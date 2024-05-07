use super::*;

pub(crate) enum ConnCmd {
    SigRecv(tx5_signal::SignalMessage),
    #[allow(dead_code)]
    Close,
}

/// Receive messages from a tx5 connection.
pub struct ConnRecv(tokio::sync::mpsc::Receiver<Vec<u8>>);

impl ConnRecv {
    /// Receive up to 16KiB of message data.
    pub async fn recv(&mut self) -> Option<Vec<u8>> {
        self.0.recv().await
    }
}

/// A tx5 connection.
pub struct Conn {
    ready: Arc<tokio::sync::Semaphore>,
    pub_key: PubKey,
    client: Weak<tx5_signal::SignalConnection>,
    conn_task: tokio::task::JoinHandle<()>,
    keepalive_task: tokio::task::JoinHandle<()>,
}

impl Drop for Conn {
    fn drop(&mut self) {
        self.conn_task.abort();
        self.keepalive_task.abort();
    }
}

impl Conn {
    #[cfg(test)]
    pub(crate) fn test_kill_keepalive_task(&self) {
        self.keepalive_task.abort();
    }

    pub(crate) fn priv_new(
        pub_key: PubKey,
        client: Weak<tx5_signal::SignalConnection>,
        config: Arc<tx5_signal::SignalConfig>,
    ) -> (Arc<Self>, ConnRecv, Arc<tokio::sync::mpsc::Sender<ConnCmd>>) {
        // zero len semaphore.. we actually just wait for the close
        let ready = Arc::new(tokio::sync::Semaphore::new(0));

        let (msg_send, msg_recv) = tokio::sync::mpsc::channel(32);
        let (cmd_send, mut cmd_recv) = tokio::sync::mpsc::channel(32);
        let cmd_send = Arc::new(cmd_send);

        let keepalive_dur = config.max_idle / 2;
        let client2 = client.clone();
        let pub_key2 = pub_key.clone();
        let keepalive_task = tokio::task::spawn(async move {
            loop {
                tokio::time::sleep(keepalive_dur).await;

                if let Some(client) = client2.upgrade() {
                    if client.send_keepalive(&pub_key2).await.is_err() {
                        break;
                    }
                } else {
                    break;
                }
            }
        });

        let ready2 = ready.clone();
        let client2 = client.clone();
        let pub_key2 = pub_key.clone();
        let conn_task = tokio::task::spawn(async move {
            let client = match client2.upgrade() {
                Some(client) => client,
                None => return,
            };

            let handshake_fut = async {
                let nonce = client.send_handshake_req(&pub_key2).await?;

                let mut got_peer_res = false;
                let mut sent_our_res = false;

                while let Some(cmd) = cmd_recv.recv().await {
                    match cmd {
                        ConnCmd::SigRecv(sig) => {
                            use tx5_signal::SignalMessage::*;
                            match sig {
                                HandshakeReq(oth_nonce) => {
                                    client
                                        .send_handshake_res(
                                            &pub_key2, oth_nonce,
                                        )
                                        .await?;
                                    sent_our_res = true;
                                }
                                HandshakeRes(res_nonce) => {
                                    if res_nonce != nonce {
                                        return Err(Error::other(
                                            "nonce mismatch",
                                        ));
                                    }
                                    got_peer_res = true;
                                }
                                _ => {
                                    return Err(Error::other(
                                        "invalid message during handshake",
                                    ));
                                }
                            }
                        }
                        ConnCmd::Close => {
                            return Err(Error::other("close during handshake"))
                        }
                    }
                    if got_peer_res && sent_our_res {
                        break;
                    }
                }

                Result::Ok(())
            };

            match tokio::time::timeout(config.max_idle, handshake_fut).await {
                Err(_) | Ok(Err(_)) => {
                    client.close_peer(&pub_key2).await;
                    return;
                }
                Ok(Ok(_)) => (),
            }

            drop(client);

            // closing the semaphore causes all the acquire awaits to end
            ready2.close();

            while let Ok(Some(cmd)) =
                tokio::time::timeout(config.max_idle, cmd_recv.recv()).await
            {
                match cmd {
                    ConnCmd::SigRecv(sig) => {
                        use tx5_signal::SignalMessage::*;
                        #[allow(clippy::single_match)] // placeholder
                        match sig {
                            // invalid
                            HandshakeReq(_) | HandshakeRes(_) => break,
                            Message(msg) => {
                                if msg_send.send(msg).await.is_err() {
                                    break;
                                }
                            }
                            _ => (),
                        }
                    }
                    ConnCmd::Close => break,
                }
            }

            // explicitly close the peer
            if let Some(client) = client2.upgrade() {
                client.close_peer(&pub_key2).await;
            };

            // the receiver side is closed because msg_send is dropped.
        });

        (
            Arc::new(Self {
                ready,
                pub_key,
                client,
                conn_task,
                keepalive_task,
            }),
            ConnRecv(msg_recv),
            cmd_send,
        )
    }

    /// Wait until this connection is ready to send / receive data.
    pub async fn ready(&self) {
        // this will error when we close the semaphore waking up the task
        let _ = self.ready.acquire().await;
    }

    /// The pub key of the remote peer this is connected to.
    pub fn pub_key(&self) -> &PubKey {
        &self.pub_key
    }

    /// Send up to 16KiB of message data.
    pub async fn send(&self, msg: Vec<u8>) -> Result<()> {
        if let Some(client) = self.client.upgrade() {
            client.send_message(&self.pub_key, msg).await
        } else {
            Err(Error::other("closed"))
        }
    }
}
