use crate::*;

enum Cmd {
    Recv(Vec<u8>),
    AwaitPermit {
        await_registered: tokio::sync::oneshot::Sender<()>,
        got_permit: tokio::sync::oneshot::Sender<()>,
    },
    RemotePermit(tokio::sync::OwnedSemaphorePermit, u32),
    Close,
}

/// Receive a framed message on the connection.
pub struct FramedConnRecv(tokio::sync::mpsc::Receiver<Vec<u8>>);

impl FramedConnRecv {
    /// Receive a framed message on the connection.
    pub async fn recv(&mut self) -> Option<Vec<u8>> {
        self.0.recv().await
    }
}

/// A framed wrapper that can send and receive larger messages than
/// the base connection.
///
/// If a message is under the frame limit, it is just sent.
///
/// If a message is OVER the frame limit, we instead request a permit
/// to send the frame count to the remote peer. We only begin sending frames
/// once we receive the permit to do so.
///
/// This allows individual peers to throttle the amount of pending-completion
/// message memory they are allocating by only issuing permits up to a
/// configured memory threshold.
pub struct FramedConn {
    pub_key: PubKey,
    weak_conn: Weak<Conn>,
    conn: tokio::sync::Mutex<Arc<Conn>>,
    cmd_send: tokio::sync::mpsc::Sender<Cmd>,
    recv_task: tokio::task::JoinHandle<()>,
    cmd_task: tokio::task::JoinHandle<()>,
}

impl Drop for FramedConn {
    fn drop(&mut self) {
        self.recv_task.abort();
        self.cmd_task.abort();
    }
}

impl FramedConn {
    /// Construct a new framed wrapper around the base connection.
    pub async fn new(
        conn: Arc<Conn>,
        mut conn_recv: ConnRecv,
        recv_limit: Arc<tokio::sync::Semaphore>,
    ) -> Result<(Self, FramedConnRecv)> {
        conn.ready().await;

        // send the protocol header
        let (a, b, c, d) = crate::proto::PROTO_VER_2.encode()?;
        conn.send(vec![a, b, c, d]).await?;

        let (cmd_send, mut cmd_recv) = tokio::sync::mpsc::channel(32);
        let (msg_send, msg_recv) = tokio::sync::mpsc::channel(32);

        // set up the receive to just feed straight into the cmd task
        let cmd_send2 = cmd_send.clone();
        let recv_task = tokio::task::spawn(async move {
            while let Some(msg) = conn_recv.recv().await {
                if cmd_send2.send(Cmd::Recv(msg)).await.is_err() {
                    break;
                }
            }

            let _ = cmd_send2.send(Cmd::Close).await;
        });

        let pub_key = conn.pub_key().clone();

        // set up the cmd task.
        // this is the main event loop of the framed wrapper
        let pub_key2 = pub_key.clone();
        let cmd_send2 = cmd_send.clone();
        let weak_conn = Arc::downgrade(&conn);
        let cmd_task = tokio::task::spawn(async move {
            // init the stateful protocol decoder
            let mut dec = crate::proto::ProtoDecoder::default();

            while let Some(cmd) = cmd_recv.recv().await {
                match cmd {
                    Cmd::Recv(msg) => {
                        use crate::proto::ProtoDecodeResult::*;
                        match dec.decode(&msg) {
                            Err(_) => break,
                            Ok(Idle) => (),
                            Ok(Message(msg)) => {
                                // received a message, forward to receiver
                                tracing::trace!(
                                    target: "NETAUDIT",
                                    pub_key = ?pub_key2,
                                    byte_count = msg.len(),
                                    m = "tx5-connection",
                                    a = "recv_framed",
                                );
                                if msg_send.send(msg).await.is_err() {
                                    tracing::info!("FramedConnRecv closed, stopping cmd task");
                                    break;
                                }
                            }
                            Ok(RemotePermitRequest(permit_len)) => {
                                // receive a permit request,
                                // await the semaphore outside this loop,
                                // the semaphore permits will be issued
                                // in the order the request come in
                                let recv_limit = recv_limit.clone();
                                let cmd_send = cmd_send2.clone();
                                // fire and forget
                                tokio::task::spawn(async move {
                                    if let Ok(permit) = recv_limit
                                        .acquire_many_owned(permit_len)
                                        .await
                                    {
                                        let _ = cmd_send
                                            .send(Cmd::RemotePermit(
                                                permit, permit_len,
                                            ))
                                            .await;
                                    }
                                });
                            }
                            Ok(RemotePermitGrant(_)) => (),
                        }
                    }
                    Cmd::AwaitPermit {
                        await_registered,
                        got_permit,
                    } => {
                        // register a oneshot to be triggered when a
                        // permit request is responded to

                        // we register the oneshot request with the
                        // stateful decoder
                        if dec
                            .sent_remote_permit_request(Some(got_permit))
                            .is_err()
                        {
                            break;
                        }

                        // we also notify the caller that we registered it
                        let _ = await_registered.send(());
                    }
                    Cmd::RemotePermit(permit, permit_len) => {
                        // our semaphore has granted a permit for the
                        // remote peer to begin sending us data
                        // now we need to notify them of that fact

                        // our stateful decoder also needs to know about it
                        if dec.sent_remote_permit_grant(permit).is_err() {
                            break;
                        }

                        // now notify our peer
                        if let Some(conn) = weak_conn.upgrade() {
                            let (a, b, c, d) =
                                match crate::proto::ProtoHeader::PermitGrant(
                                    permit_len,
                                )
                                .encode()
                                {
                                    Ok(r) => r,
                                    Err(_) => break,
                                };
                            if conn.send(vec![a, b, c, d]).await.is_err() {
                                break;
                            }
                        } else {
                            break;
                        }
                    }
                    Cmd::Close => break,
                }
            }
        });

        Ok((
            Self {
                pub_key,
                weak_conn: Arc::downgrade(&conn),
                conn: tokio::sync::Mutex::new(conn),
                cmd_send,
                recv_task,
                cmd_task,
            },
            FramedConnRecv(msg_recv),
        ))
    }

    /// The pub key of the remote peer this is connected to.
    pub fn pub_key(&self) -> &PubKey {
        &self.pub_key
    }

    /// Returns `true` if we sucessfully connected over webrtc.
    pub fn is_using_webrtc(&self) -> bool {
        if let Some(conn) = self.weak_conn.upgrade() {
            conn.is_using_webrtc()
        } else {
            false
        }
    }

    /// Get connection statistics.
    pub fn get_stats(&self) -> ConnStats {
        if let Some(conn) = self.weak_conn.upgrade() {
            conn.get_stats()
        } else {
            ConnStats::default()
        }
    }

    /// Send a message on the connection.
    pub async fn send(&self, msg: Vec<u8>) -> Result<()> {
        let byte_count = msg.len();
        match self.send_inner(msg).await {
            Ok(_) => {
                tracing::trace!(
                    target: "NETAUDIT",
                    pub_key = ?self.pub_key,
                    byte_count,
                    m = "tx5-connection",
                    a = "send_framed_success",
                );
                Ok(())
            }
            Err(err) => {
                tracing::debug!(
                    target: "NETAUDIT",
                    pub_key = ?self.pub_key,
                    byte_count,
                    ?err,
                    m = "tx5-connection",
                    a = "send_framed_error",
                );
                Err(err)
            }
        }
    }

    /// Helper to do the sending, breaking up the messages if needed and
    /// awaiting the permit before sending broken up messages.
    async fn send_inner(&self, msg: Vec<u8>) -> Result<()> {
        let conn = self.conn.lock().await;

        match crate::proto::proto_encode(&msg)? {
            crate::proto::ProtoEncodeResult::OneMessage(msg) => {
                // it's a small message, just send it as one chunk
                conn.send(msg).await?;
            }
            crate::proto::ProtoEncodeResult::NeedPermit {
                permit_req,
                msg_payload,
            } => {
                // it's a big message, we've got chunks

                let (s_reg, r_reg) = tokio::sync::oneshot::channel();
                let (s_perm, r_perm) = tokio::sync::oneshot::channel();

                // coordinate with the cmd task that we need a permit
                self.cmd_send
                    .send(Cmd::AwaitPermit {
                        await_registered: s_reg,
                        got_permit: s_perm,
                    })
                    .await
                    .map_err(|_| Error::other("closed"))?;

                // wait for the want permit to be registered
                r_reg.await.map_err(|_| Error::other("closed"))?;

                // send the permit request
                conn.send(permit_req).await?;

                // wait for the permit to be authorized by the peer
                r_perm.await.map_err(|_| Error::other("closed"))?;

                // send the chunked messages
                for msg in msg_payload {
                    conn.send(msg).await?;
                }
            }
        }

        Ok(())
    }
}
