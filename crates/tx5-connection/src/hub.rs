pub use super::*;

type HubMapT = HashMap<PubKey, (Weak<Conn>, CloseSend<ConnCmd>)>;
struct HubMap(HubMapT);

impl std::ops::Deref for HubMap {
    type Target = HubMapT;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for HubMap {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl HubMap {
    pub fn new() -> Self {
        Self(HashMap::new())
    }
}

async fn hub_map_assert(
    is_polite: bool,
    pub_key: PubKey,
    map: &mut HubMap,
    client: &Arc<tx5_signal::SignalConnection>,
    config: &Arc<tx5_signal::SignalConfig>,
) -> Result<(Option<ConnRecv>, Arc<Conn>, CloseSend<ConnCmd>)> {
    let mut found_during_prune = None;

    map.retain(|_, c| {
        if let Some(conn) = c.0.upgrade() {
            let cmd_send = c.1.clone();
            if conn.pub_key() == &pub_key {
                found_during_prune = Some((conn, cmd_send));
            }
            true
        } else {
            false
        }
    });

    if let Some((conn, cmd_send)) = found_during_prune {
        return Ok((None, conn, cmd_send));
    }

    client.assert(&pub_key).await?;

    // we're connected to the peer, create a connection

    let (conn, recv, cmd_send) = Conn::priv_new(
        is_polite,
        pub_key.clone(),
        Arc::downgrade(client),
        config.clone(),
    );

    let weak_conn = Arc::downgrade(&conn);

    let mut store_cmd_send = cmd_send.clone();
    store_cmd_send.set_close_on_drop(true);

    map.insert(pub_key, (weak_conn, store_cmd_send));

    Ok((Some(recv), conn, cmd_send))
}

pub(crate) enum HubCmd {
    CliRecv {
        pub_key: PubKey,
        msg: tx5_signal::SignalMessage,
    },
    Connect {
        pub_key: PubKey,
        resp:
            tokio::sync::oneshot::Sender<Result<(Option<ConnRecv>, Arc<Conn>)>>,
    },
    Close,
}

/// A stream of incoming p2p connections.
pub struct HubRecv(tokio::sync::mpsc::Receiver<(Arc<Conn>, ConnRecv)>);

impl HubRecv {
    /// Receive an incoming p2p connection.
    pub async fn accept(&mut self) -> Option<(Arc<Conn>, ConnRecv)> {
        self.0.recv().await
    }
}

/// A signal server connection from which we can establish tx5 connections.
pub struct Hub {
    client: Arc<tx5_signal::SignalConnection>,
    cmd_send: tokio::sync::mpsc::Sender<HubCmd>,
    task_list: Vec<tokio::task::JoinHandle<()>>,
}

impl Drop for Hub {
    fn drop(&mut self) {
        for task in self.task_list.iter() {
            task.abort();
        }
    }
}

impl Hub {
    /// Create a new Hub based off a connected tx5 signal client.
    /// Note, if this is not a "listener" client,
    /// you do not need to ever call accept.
    pub async fn new(
        url: &str,
        config: Arc<tx5_signal::SignalConfig>,
    ) -> Result<(Self, HubRecv)> {
        let (client, mut recv) =
            tx5_signal::SignalConnection::connect(url, config.clone()).await?;
        let client = Arc::new(client);

        tracing::debug!(%url, pub_key = ?client.pub_key(), "hub connected");

        let mut task_list = Vec::new();

        let (cmd_send, mut cmd_recv) = tokio::sync::mpsc::channel(32);

        let cmd_send2 = cmd_send.clone();
        task_list.push(tokio::task::spawn(async move {
            while let Some((pub_key, msg)) = recv.recv_message().await {
                if cmd_send2
                    .send(HubCmd::CliRecv { pub_key, msg })
                    .await
                    .is_err()
                {
                    break;
                }
            }

            let _ = cmd_send2.send(HubCmd::Close).await;
        }));

        let (conn_send, conn_recv) = tokio::sync::mpsc::channel(32);
        let weak_client = Arc::downgrade(&client);
        let url = url.to_string();
        let this_pub_key = client.pub_key().clone();
        task_list.push(tokio::task::spawn(async move {
            let mut map = HubMap::new();
            while let Some(cmd) = cmd_recv.recv().await {
                match cmd {
                    HubCmd::CliRecv { pub_key, msg } => {
                        if let Some(client) = weak_client.upgrade() {
                            if pub_key == this_pub_key {
                                // ignore self messages
                                continue;
                            }
                            let is_polite = pub_key > this_pub_key;
                            let (recv, conn, cmd_send) = match hub_map_assert(
                                is_polite, pub_key, &mut map, &client, &config,
                            )
                            .await
                            {
                                Err(err) => {
                                    tracing::debug!(
                                        ?err,
                                        "failed to accept incoming connection"
                                    );
                                    continue;
                                }
                                Ok(r) => r,
                            };
                            let _ = cmd_send.send(ConnCmd::SigRecv(msg)).await;
                            if let Some(recv) = recv {
                                let _ = conn_send.send((conn, recv)).await;
                            }
                        } else {
                            break;
                        }
                    }
                    HubCmd::Connect { pub_key, resp } => {
                        if pub_key == this_pub_key {
                            let _ = resp.send(Err(Error::other(
                                "cannot connect to self",
                            )));
                            continue;
                        }
                        let is_polite = pub_key > this_pub_key;
                        if let Some(client) = weak_client.upgrade() {
                            let _ = resp.send(
                                hub_map_assert(
                                    is_polite, pub_key, &mut map, &client,
                                    &config,
                                )
                                .await
                                .map(|(recv, conn, _)| (recv, conn)),
                            );
                        } else {
                            break;
                        }
                    }
                    HubCmd::Close => break,
                }
            }

            if let Some(client) = weak_client.upgrade() {
                client.close().await;
            }

            tracing::debug!(%url, ?this_pub_key, "hub close");
        }));

        Ok((
            Self {
                client,
                cmd_send,
                task_list,
            },
            HubRecv(conn_recv),
        ))
    }

    /// Get the pub_key used by this hub.
    pub fn pub_key(&self) -> &PubKey {
        self.client.pub_key()
    }

    /// Establish a connection to a remote peer.
    pub async fn connect(
        &self,
        pub_key: PubKey,
    ) -> Result<(Arc<Conn>, ConnRecv)> {
        let (s, r) = tokio::sync::oneshot::channel();
        self.cmd_send
            .send(HubCmd::Connect { pub_key, resp: s })
            .await
            .map_err(|_| Error::other("closed"))?;
        let (recv, conn) = r.await.map_err(|_| Error::other("closed"))??;
        if let Some(recv) = recv {
            Ok((conn, recv))
        } else {
            Err(Error::other("already connected"))
        }
    }
}
