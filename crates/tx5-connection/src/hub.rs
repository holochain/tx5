pub use super::*;

type HubMap = HashMap<PubKey, Weak<Tx5Connection>>;

async fn hub_map_assert(
    pub_key: PubKey,
    map: &mut HubMap,
    client: &Arc<tx5_signal::SignalConnection>,
) -> Result<(bool, Arc<Tx5Connection>)> {
    let mut found_during_prune = None;

    map.retain(|_, c| {
        if let Some(c) = c.upgrade() {
            found_during_prune = Some(c.clone());
            true
        } else {
            false
        }
    });

    if let Some(found) = found_during_prune {
        return Ok((false, found));
    }

    client.assert(&pub_key).await?;

    // we're connected to the peer, create a connection

    let conn = Tx5Connection::priv_new(pub_key.clone(), Arc::downgrade(client));

    let weak_conn = Arc::downgrade(&conn);

    map.insert(pub_key, weak_conn);

    Ok((true, conn))
}

enum HubCmd {
    CliRecv {
        pub_key: PubKey,
        msg: tx5_signal::SignalMessage,
    },
    Connect {
        pub_key: PubKey,
        resp: tokio::sync::oneshot::Sender<Result<Arc<Tx5Connection>>>,
    },
}

/// A signal server connection from which we can establish tx5 connections.
pub struct Tx5ConnectionHub {
    client: Arc<tx5_signal::SignalConnection>,
    cmd_send: tokio::sync::mpsc::Sender<HubCmd>,
    conn_recv:
        tokio::sync::Mutex<tokio::sync::mpsc::Receiver<Arc<Tx5Connection>>>,
    task_list: Vec<tokio::task::JoinHandle<()>>,
}

impl Drop for Tx5ConnectionHub {
    fn drop(&mut self) {
        for task in self.task_list.iter() {
            task.abort();
        }
    }
}

impl Tx5ConnectionHub {
    /// Create a new Tx5ConnectionHub based off a connected tx5 signal client.
    /// Note, if this is not a "listener" client,
    /// you do not need to ever call accept.
    pub fn new(client: tx5_signal::SignalConnection) -> Self {
        let client = Arc::new(client);

        let mut task_list = Vec::new();

        let (cmd_send, mut cmd_recv) = tokio::sync::mpsc::channel(32);

        let cmd_send2 = cmd_send.clone();
        let weak_client = Arc::downgrade(&client);
        task_list.push(tokio::task::spawn(async move {
            loop {
                if let Some(client) = weak_client.upgrade() {
                    if let Some((pub_key, msg)) = client.recv_message().await {
                        if cmd_send2
                            .send(HubCmd::CliRecv { pub_key, msg })
                            .await
                            .is_err()
                        {
                            break;
                        }
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            }
        }));

        let (conn_send, conn_recv) = tokio::sync::mpsc::channel(32);
        let weak_client = Arc::downgrade(&client);
        task_list.push(tokio::task::spawn(async move {
            let mut map = HubMap::new();
            while let Some(cmd) = cmd_recv.recv().await {
                match cmd {
                    HubCmd::CliRecv { pub_key, msg } => {
                        if let Some(client) = weak_client.upgrade() {
                            let (did_create, conn) = match hub_map_assert(
                                pub_key, &mut map, &client,
                            )
                            .await
                            {
                                Err(_) => continue,
                                Ok(conn) => conn,
                            };
                            let _ =
                                conn.cmd_send.send(ConnCmd::SigRecv(msg)).await;
                            if did_create {
                                let _ = conn_send.send(conn).await;
                            }
                        } else {
                            break;
                        }
                    }
                    HubCmd::Connect { pub_key, resp } => {
                        if let Some(client) = weak_client.upgrade() {
                            let _ = resp.send(
                                hub_map_assert(pub_key, &mut map, &client)
                                    .await
                                    .map(|(_, conn)| conn),
                            );
                        } else {
                            break;
                        }
                    }
                }
            }
        }));

        Self {
            client,
            cmd_send,
            conn_recv: tokio::sync::Mutex::new(conn_recv),
            task_list,
        }
    }

    /// Get the pub_key used by this hub.
    pub fn pub_key(&self) -> &PubKey {
        self.client.pub_key()
    }

    /// Establish a connection to a remote peer.
    /// Note, if there is already an open connection, this Arc will point
    /// to that same connection instance.
    pub async fn connect(&self, pub_key: PubKey) -> Result<Arc<Tx5Connection>> {
        let (s, r) = tokio::sync::oneshot::channel();
        self.cmd_send
            .send(HubCmd::Connect { pub_key, resp: s })
            .await
            .map_err(|_| Error::other("closed"))?;
        r.await.map_err(|_| Error::other("closed"))?
    }

    /// Accept an incoming tx5 connection.
    pub async fn accept(&self) -> Option<Arc<Tx5Connection>> {
        self.conn_recv.lock().await.recv().await
    }
}
