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

/// Insert or update a peer connection in the hub.
async fn hub_map_assert(
    webrtc_config: &Arc<Mutex<WebRtcConfig>>,
    is_polite: bool,
    pub_key: PubKey,
    map: &mut HubMap,
    client: &Arc<tx5_signal::SignalConnection>,
    config: &Arc<HubConfig>,
    hub_cmd_send: &tokio::sync::mpsc::Sender<HubCmd>,
) -> Result<(Option<ConnRecv>, Arc<Conn>, CloseSend<ConnCmd>)> {
    let mut found_during_prune = None;

    // remove anything that has been dropped
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

    // if we happened to find what we were looking for
    // (that is still valid) in the prune loop above, short circuit here.
    if let Some((conn, cmd_send)) = found_during_prune {
        return Ok((None, conn, cmd_send));
    }

    client.assert(&pub_key).await?;

    // we're connected to the peer, create a connection

    let (conn, recv, cmd_send) = Conn::priv_new(
        webrtc_config.lock().unwrap().clone(),
        is_polite,
        pub_key.clone(),
        Arc::downgrade(client),
        config.clone(),
        hub_cmd_send.clone(),
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
    Disconnect(PubKey),
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
    webrtc_config: Arc<Mutex<WebRtcConfig>>,
    client: Arc<tx5_signal::SignalConnection>,
    hub_cmd_send: tokio::sync::mpsc::Sender<HubCmd>,
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
        webrtc_config: WebRtcConfig,
        url: &str,
        config: Arc<HubConfig>,
    ) -> Result<(Self, HubRecv)> {
        let webrtc_config = Arc::new(Mutex::new(webrtc_config));

        let (client, mut recv) = tx5_signal::SignalConnection::connect(
            url,
            config.signal_config.clone(),
        )
        .await?;
        let client = Arc::new(client);

        tracing::debug!(%url, pub_key = ?client.pub_key(), "hub connected");

        let mut task_list = Vec::new();

        let (hub_cmd_send, mut cmd_recv) = tokio::sync::mpsc::channel(32);

        // forward received messages to the cmd task
        let hub_cmd_send2 = hub_cmd_send.clone();
        task_list.push(tokio::task::spawn(async move {
            while let Some((pub_key, msg)) = recv.recv_message().await {
                if hub_cmd_send2
                    .send(HubCmd::CliRecv { pub_key, msg })
                    .await
                    .is_err()
                {
                    break;
                }
            }

            let _ = hub_cmd_send2.send(HubCmd::Close).await;
        }));

        // the cmd task is the main event loop of the hub logic
        let webrtc_config2 = webrtc_config.clone();
        let (conn_send, conn_recv) = tokio::sync::mpsc::channel(32);
        let weak_client = Arc::downgrade(&client);
        let url = url.to_string();
        let this_pub_key = client.pub_key().clone();
        let hub_cmd_send2 = hub_cmd_send.clone();
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

                            // assert a connection for this message
                            let is_polite = pub_key > this_pub_key;
                            let (recv, conn, cmd_send) = match hub_map_assert(
                                &webrtc_config2,
                                is_polite,
                                pub_key,
                                &mut map,
                                &client,
                                &config,
                                &hub_cmd_send2,
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

                            // then forward it along
                            let _ = cmd_send.send(ConnCmd::SigRecv(msg)).await;

                            // if this opened a new connection,
                            // send that event too
                            if let Some(recv) = recv {
                                let _ = conn_send.send((conn, recv)).await;
                            }
                        } else {
                            break;
                        }
                    }
                    HubCmd::Connect { pub_key, resp } => {
                        // user requested a connect without a message to send

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
                                    &webrtc_config2,
                                    is_polite,
                                    pub_key,
                                    &mut map,
                                    &client,
                                    &config,
                                    &hub_cmd_send2,
                                )
                                .await
                                .map(|(recv, conn, _)| (recv, conn)),
                            );
                        } else {
                            break;
                        }
                    }
                    HubCmd::Disconnect(pub_key) => {
                        // disconnect from a remote peer

                        if let Some(client) = weak_client.upgrade() {
                            let _ = client.close_peer(&pub_key).await;
                        } else {
                            break;
                        }
                        let _ = map.remove(&pub_key);
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
                webrtc_config,
                client,
                hub_cmd_send,
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
        self.hub_cmd_send
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

    /// Alter the webrtc_config at runtime.
    ///
    /// This will affect all future new outgoing connections, and all connections accepted on the
    /// receiver. It will not affect any connections already established.
    pub fn set_webrtc_config(&self, webrtc_config: WebRtcConfig) {
        *self.webrtc_config.lock().unwrap() = webrtc_config;
    }
}
