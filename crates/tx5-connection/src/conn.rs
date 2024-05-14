use super::*;
use std::sync::atomic::Ordering;

pub(crate) enum ConnCmd {
    SigRecv(tx5_signal::SignalMessage),
    SendMessage(Vec<u8>),
    WebrtcMessage(Vec<u8>),
    WebrtcReady,
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
    cmd_send: CloseSend<ConnCmd>,
    conn_task: tokio::task::JoinHandle<()>,
    keepalive_task: tokio::task::JoinHandle<()>,
    webrtc_ready: Arc<tokio::sync::Semaphore>,
    send_msg_count: Arc<std::sync::atomic::AtomicU64>,
    send_byte_count: Arc<std::sync::atomic::AtomicU64>,
    recv_msg_count: Arc<std::sync::atomic::AtomicU64>,
    recv_byte_count: Arc<std::sync::atomic::AtomicU64>,
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
        is_polite: bool,
        pub_key: PubKey,
        client: Weak<tx5_signal::SignalConnection>,
        config: Arc<tx5_signal::SignalConfig>,
    ) -> (Arc<Self>, ConnRecv, CloseSend<ConnCmd>) {
        let send_msg_count = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let send_byte_count = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let recv_msg_count = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let recv_byte_count = Arc::new(std::sync::atomic::AtomicU64::new(0));

        // zero len semaphore.. we actually just wait for the close
        let ready = Arc::new(tokio::sync::Semaphore::new(0));
        let webrtc_ready = Arc::new(tokio::sync::Semaphore::new(0));

        let (mut msg_send, msg_recv) = CloseSend::channel();
        let (cmd_send, mut cmd_recv) = CloseSend::channel();

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

        let send_msg_count2 = send_msg_count.clone();
        let send_byte_count2 = send_byte_count.clone();
        let recv_msg_count2 = recv_msg_count.clone();
        let recv_byte_count2 = recv_byte_count.clone();
        let webrtc_ready2 = webrtc_ready.clone();
        let ready2 = ready.clone();
        let client2 = client.clone();
        let pub_key2 = pub_key.clone();
        let cmd_send3 = cmd_send.clone();
        let conn_task = tokio::task::spawn(async move {
            let client = match client2.upgrade() {
                Some(client) => client,
                None => return,
            };

            let mut webrtc_message_buffer = Vec::new();

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
                                // Ignore all other message types...
                                // they may be from previous sessions
                                _ => (),
                            }
                        }
                        ConnCmd::SendMessage(_) => {
                            return Err(Error::other("send before ready"));
                        }
                        ConnCmd::WebrtcMessage(msg) => {
                            webrtc_message_buffer.push(msg);
                        }
                        ConnCmd::WebrtcReady => {
                            webrtc_ready2.close();
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

            let (webrtc, mut webrtc_recv) = webrtc::Webrtc::new(
                is_polite,
                // TODO - pass stun server config here
                b"{}".to_vec(),
                // TODO - make this configurable
                4096,
            );

            let client3 = client2.clone();
            let pub_key3 = pub_key2.clone();
            let _webrtc_task = AbortTask(tokio::task::spawn(async move {
                use webrtc::WebrtcEvt::*;
                while let Some(evt) = webrtc_recv.recv().await {
                    match evt {
                        GeneratedOffer(offer) => {
                            if let Some(client) = client3.upgrade() {
                                if client
                                    .send_offer(&pub_key3, offer)
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                            } else {
                                break;
                            }
                        }
                        GeneratedAnswer(answer) => {
                            if let Some(client) = client3.upgrade() {
                                if client
                                    .send_answer(&pub_key3, answer)
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                            } else {
                                break;
                            }
                        }
                        GeneratedIce(ice) => {
                            if let Some(client) = client3.upgrade() {
                                if client
                                    .send_ice(&pub_key3, ice)
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                            } else {
                                break;
                            }
                        }
                        Message(msg) => {
                            if cmd_send3
                                .send(ConnCmd::WebrtcMessage(msg))
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                        Ready => {
                            if cmd_send3
                                .send(ConnCmd::WebrtcReady)
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                    }
                }
            }));

            msg_send.set_close_on_drop(true);

            let mut recv_over_webrtc = false;
            let mut send_over_webrtc = false;

            while let Ok(Some(cmd)) =
                tokio::time::timeout(config.max_idle, cmd_recv.recv()).await
            {
                match cmd {
                    ConnCmd::SigRecv(sig) => {
                        use tx5_signal::SignalMessage::*;
                        match sig {
                            // invalid
                            HandshakeReq(_) | HandshakeRes(_) => break,
                            Message(msg) => {
                                if recv_over_webrtc {
                                    // invalid signal message
                                    // after webrtc ready received
                                    break;
                                } else {
                                    recv_msg_count2
                                        .fetch_add(1, Ordering::Relaxed);
                                    recv_byte_count2.fetch_add(
                                        msg.len() as u64,
                                        Ordering::Relaxed,
                                    );
                                    if msg_send.send(msg).await.is_err() {
                                        break;
                                    }
                                }
                            }
                            Offer(offer) => {
                                if webrtc.in_offer(offer).await.is_err() {
                                    break;
                                }
                            }
                            Answer(answer) => {
                                if webrtc.in_answer(answer).await.is_err() {
                                    break;
                                }
                            }
                            Ice(ice) => {
                                if webrtc.in_ice(ice).await.is_err() {
                                    break;
                                }
                            }
                            WebrtcReady => {
                                recv_over_webrtc = true;
                                for msg in webrtc_message_buffer.drain(..) {
                                    // don't bump send metrics here,
                                    // we bumped them on receive
                                    if msg_send.send(msg).await.is_err() {
                                        break;
                                    }
                                }
                            }
                            _ => (),
                        }
                    }
                    ConnCmd::SendMessage(msg) => {
                        send_msg_count2.fetch_add(1, Ordering::Relaxed);
                        send_byte_count2
                            .fetch_add(msg.len() as u64, Ordering::Relaxed);
                        if send_over_webrtc {
                            if webrtc.message(msg).await.is_err() {
                                break;
                            }
                        } else if let Some(client) = client2.upgrade() {
                            if client
                                .send_message(&pub_key2, msg)
                                .await
                                .is_err()
                            {
                                break;
                            }
                        } else {
                            break;
                        }
                    }
                    ConnCmd::WebrtcMessage(msg) => {
                        recv_msg_count2.fetch_add(1, Ordering::Relaxed);
                        recv_byte_count2
                            .fetch_add(msg.len() as u64, Ordering::Relaxed);
                        if recv_over_webrtc {
                            if msg_send.send(msg).await.is_err() {
                                break;
                            }
                        } else {
                            webrtc_message_buffer.push(msg);
                            if webrtc_message_buffer.len() > 32 {
                                // prevent memory fillup
                                break;
                            }
                        }
                    }
                    ConnCmd::WebrtcReady => {
                        if let Some(client) = client2.upgrade() {
                            if client
                                .send_webrtc_ready(&pub_key2)
                                .await
                                .is_err()
                            {
                                break;
                            }
                        } else {
                            break;
                        }
                        send_over_webrtc = true;
                        webrtc_ready2.close();
                    }
                }
            }

            // explicitly close the peer
            if let Some(client) = client2.upgrade() {
                client.close_peer(&pub_key2).await;
            };

            // the receiver side is closed because msg_send is dropped.
        });

        let mut cmd_send2 = cmd_send.clone();
        cmd_send2.set_close_on_drop(true);
        let this = Self {
            ready,
            pub_key,
            cmd_send: cmd_send2,
            conn_task,
            keepalive_task,
            webrtc_ready,
            send_msg_count,
            send_byte_count,
            recv_msg_count,
            recv_byte_count,
        };

        (Arc::new(this), ConnRecv(msg_recv), cmd_send)
    }

    /// Wait until this connection is ready to send / receive data.
    pub async fn ready(&self) {
        // this will error when we close the semaphore waking up the task
        let _ = self.ready.acquire().await;
    }

    /// Wait until this connection is connected via webrtc.
    /// Note, this will never resolve if we never successfully
    /// connect over webrtc.
    pub async fn webrtc_ready(&self) {
        let _ = self.webrtc_ready.acquire().await;
    }

    /// Returns `true` if we sucessfully connected over webrtc.
    pub fn is_using_webrtc(&self) -> bool {
        self.webrtc_ready.is_closed()
    }

    /// The pub key of the remote peer this is connected to.
    pub fn pub_key(&self) -> &PubKey {
        &self.pub_key
    }

    /// Send up to 16KiB of message data.
    pub async fn send(&self, msg: Vec<u8>) -> Result<()> {
        self.cmd_send.send(ConnCmd::SendMessage(msg)).await
    }

    /// Get connection statistics.
    pub fn get_stats(&self) -> ConnStats {
        ConnStats {
            send_msg_count: self.send_msg_count.load(Ordering::Relaxed),
            send_byte_count: self.send_byte_count.load(Ordering::Relaxed),
            recv_msg_count: self.recv_msg_count.load(Ordering::Relaxed),
            recv_byte_count: self.recv_byte_count.load(Ordering::Relaxed),
        }
    }
}

/// Connection statistics.
#[derive(Default)]
pub struct ConnStats {
    /// message count sent.
    pub send_msg_count: u64,

    /// byte count sent.
    pub send_byte_count: u64,

    /// message count received.
    pub recv_msg_count: u64,

    /// byte count received.
    pub recv_byte_count: u64,
}
