use crate::*;

enum Cmd {
    Recv(Vec<u8>),
    AwaitPermit {
        await_registered: tokio::sync::oneshot::Sender<()>,
        got_permit: tokio::sync::oneshot::Sender<()>,
    },
    RemotePermit(tokio::sync::OwnedSemaphorePermit, u32),
}

/// A framed wrapper that can send and receive larger messages than
/// the base connection.
pub struct Tx5ConnFramed {
    pub_key: PubKey,
    conn: tokio::sync::Mutex<Arc<Tx5Connection>>,
    cmd_send: tokio::sync::mpsc::Sender<Cmd>,
    recv_task: tokio::task::JoinHandle<()>,
    cmd_task: tokio::task::JoinHandle<()>,
    msg_recv: tokio::sync::Mutex<tokio::sync::mpsc::Receiver<Vec<u8>>>,
}

impl Drop for Tx5ConnFramed {
    fn drop(&mut self) {
        self.recv_task.abort();
        self.cmd_task.abort();
    }
}

impl Tx5ConnFramed {
    /// Construct a new framed wrapper around the base connection.
    pub async fn new(
        conn: Arc<Tx5Connection>,
        recv_limit: Arc<tokio::sync::Semaphore>,
    ) -> Result<Self> {
        conn.ready().await;

        let (a, b, c, d) = crate::proto::PROTO_VER_2.encode()?;
        conn.send(vec![a, b, c, d]).await?;

        let (cmd_send, mut cmd_recv) = tokio::sync::mpsc::channel(32);
        let (msg_send, msg_recv) = tokio::sync::mpsc::channel(32);

        let cmd_send2 = cmd_send.clone();
        let weak_conn = Arc::downgrade(&conn);
        let recv_task = tokio::task::spawn(async move {
            while let Some(conn) = weak_conn.upgrade() {
                if let Some(msg) = conn.recv().await {
                    if cmd_send2.send(Cmd::Recv(msg)).await.is_err() {
                        break;
                    }
                } else {
                    break;
                }
            }
        });

        let cmd_send2 = cmd_send.clone();
        let weak_conn = Arc::downgrade(&conn);
        let cmd_task = tokio::task::spawn(async move {
            let mut dec = crate::proto::ProtoDecoder::default();

            while let Some(cmd) = cmd_recv.recv().await {
                match cmd {
                    Cmd::Recv(msg) => {
                        use crate::proto::ProtoDecodeResult::*;
                        match dec.decode(&msg) {
                            Err(_) => break,
                            Ok(Idle) => (),
                            Ok(Message(msg)) => {
                                if msg_send.send(msg).await.is_err() {
                                    break;
                                }
                            }
                            Ok(RemotePermitRequest(permit_len)) => {
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
                        if dec
                            .sent_remote_permit_request(Some(got_permit))
                            .is_err()
                        {
                            break;
                        }
                        let _ = await_registered.send(());
                    }
                    Cmd::RemotePermit(permit, permit_len) => {
                        if dec.sent_remote_permit_grant(permit).is_err() {
                            break;
                        }
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
                }
            }
        });

        let pub_key = conn.pub_key().clone();

        Ok(Self {
            pub_key,
            conn: tokio::sync::Mutex::new(conn),
            cmd_send,
            recv_task,
            cmd_task,
            msg_recv: tokio::sync::Mutex::new(msg_recv),
        })
    }

    /// The pub key of the remote peer this is connected to.
    pub fn pub_key(&self) -> &PubKey {
        &self.pub_key
    }

    /// Receive a message on the connection.
    pub async fn recv(&self) -> Option<Vec<u8>> {
        self.msg_recv.lock().await.recv().await
    }

    /// Send a message on the connection.
    pub async fn send(&self, msg: Vec<u8>) -> Result<()> {
        let conn = self.conn.lock().await;

        match crate::proto::proto_encode(&msg)? {
            crate::proto::ProtoEncodeResult::OneMessage(msg) => {
                conn.send(msg).await?;
            }
            crate::proto::ProtoEncodeResult::NeedPermit {
                permit_req,
                msg_payload,
            } => {
                let (s_reg, r_reg) = tokio::sync::oneshot::channel();
                let (s_perm, r_perm) = tokio::sync::oneshot::channel();

                self.cmd_send
                    .send(Cmd::AwaitPermit {
                        await_registered: s_reg,
                        got_permit: s_perm,
                    })
                    .await
                    .map_err(|_| Error::other("closed"))?;

                r_reg.await.map_err(|_| Error::other("closed"))?;

                conn.send(permit_req).await?;

                r_perm.await.map_err(|_| Error::other("closed"))?;

                for msg in msg_payload {
                    conn.send(msg).await?;
                }
            }
        }

        Ok(())
    }
}
