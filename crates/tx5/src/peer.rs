use crate::*;

use tx5_connection::*;

enum MaybeReady {
    Ready(Arc<DynBackCon>),
    Wait(Arc<tokio::sync::Semaphore>),
}

/// This struct represents a connection to an individual peer.
/// It is a pretty thin wrapper around the tx5-connection "Conn".
pub(crate) struct Peer {
    ready: Arc<Mutex<MaybeReady>>,
    task: tokio::task::JoinHandle<()>,
    pub(crate) pub_key: PubKey,
    pub(crate) opened_at_s: u64,
}

fn timestamp() -> u64 {
    std::time::SystemTime::UNIX_EPOCH
        .elapsed()
        .expect("failed to get time")
        .as_secs()
}

impl Drop for Peer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Peer {
    /// Call this when we are establishing a new outgoing connection.
    pub fn new_connect(
        config: Arc<Config>,
        recv_limit: Arc<tokio::sync::Semaphore>,
        ep: Weak<Mutex<EpInner>>,
        peer_url: PeerUrl,
        evt_send: tokio::sync::mpsc::Sender<EndpointEvent>,
    ) -> Arc<Self> {
        Arc::new_cyclic(|_this| {
            let wait = Arc::new(tokio::sync::Semaphore::new(0));
            let ready = Arc::new(Mutex::new(MaybeReady::Wait(wait)));
            let pub_key = peer_url.pub_key().clone();

            let task = tokio::task::spawn(connect(
                config,
                recv_limit,
                ep,
                peer_url,
                evt_send,
                ready.clone(),
            ));

            let opened_at_s = timestamp();

            // This returns a new peer instance but the task is still working. The connection may
            // fail to be set up here.
            Self {
                ready,
                task,
                pub_key,
                opened_at_s,
            }
        })
    }

    /// Call this when we are accepting a new incoming connection.
    pub fn new_accept(
        config: Arc<Config>,
        recv_limit: Arc<tokio::sync::Semaphore>,
        ep: Weak<Mutex<EpInner>>,
        peer_url: PeerUrl,
        wc: DynBackWaitCon,
        evt_send: tokio::sync::mpsc::Sender<EndpointEvent>,
    ) -> Arc<Self> {
        Arc::new_cyclic(|_this| {
            let wait = Arc::new(tokio::sync::Semaphore::new(0));
            let ready = Arc::new(Mutex::new(MaybeReady::Wait(wait)));
            let pub_key = peer_url.pub_key().clone();

            let task = tokio::task::spawn(task(
                config,
                recv_limit,
                ep,
                Some(wc),
                peer_url,
                evt_send,
                ready.clone(),
            ));

            let opened_at_s = timestamp();

            Self {
                ready,
                task,
                pub_key,
                opened_at_s,
            }
        })
    }

    /// This is initially false, but can transition into true.
    /// If establishing a webrtc connection fails, it will remain false.
    pub fn is_using_webrtc(&self) -> bool {
        if let MaybeReady::Ready(r) = &*self.ready.lock().unwrap() {
            r.is_using_webrtc()
        } else {
            false
        }
    }

    /// Get connection statistics.
    pub fn get_stats(&self) -> ConnStats {
        if let MaybeReady::Ready(r) = &*self.ready.lock().unwrap() {
            r.get_stats()
        } else {
            ConnStats::default()
        }
    }

    /// This future resolves when the connection is ready to use.
    pub async fn ready(&self) {
        let w = match &*self.ready.lock().unwrap() {
            MaybeReady::Ready(_) => return,
            MaybeReady::Wait(w) => w.clone(),
        };

        let _ = w.acquire().await;
    }

    /// Send data to the remote peer over this connection.
    pub async fn send(&self, msg: Vec<u8>) -> Result<()> {
        let conn = match &*self.ready.lock().unwrap() {
            MaybeReady::Ready(c) => c.clone(),
            _ => return Err(Error::other("not ready")),
        };
        conn.send(msg).await
    }
}

/// Establish an outgoing connection. Once connected, this function
/// will delegate to the main event-loop "task" function below.
async fn connect(
    config: Arc<Config>,
    recv_limit: Arc<tokio::sync::Semaphore>,
    ep: Weak<Mutex<EpInner>>,
    peer_url: PeerUrl,
    evt_send: tokio::sync::mpsc::Sender<EndpointEvent>,
    ready: Arc<Mutex<MaybeReady>>,
) {
    tracing::trace!(?peer_url, "peer try connect");

    let conn = if let Some(ep) = ep.upgrade() {
        let connect_fut = async {
            let sig =
                ep.lock()
                    .unwrap()
                    .assert_sig(peer_url.to_sig(), false, None);
            sig.ready().await;
            sig.connect(peer_url.pub_key().clone()).await
        };

        // Try to initiate connection negotiation with the remote peer, with a timeout.
        match tokio::time::timeout(config.timeout, connect_fut)
            .await
            .map_err(Error::other)
        {
            Ok(Ok(conn)) => Some(conn),
            Err(err) | Ok(Err(err)) => {
                tracing::debug!(?err, "peer connect error");

                // The connection attempt to the remote peer failed or timed out, so we proceed
                // without a connection.
                None
            }
        }
    } else {
        None
    };

    task(config, recv_limit, ep, conn, peer_url, evt_send, ready).await;
}

/// Drop guard to manage cleanup incase the main event loop task exits.
struct DropPeer {
    ep: Weak<Mutex<EpInner>>,
    peer_url: PeerUrl,
    evt_send: tokio::sync::mpsc::Sender<EndpointEvent>,
}

impl Drop for DropPeer {
    fn drop(&mut self) {
        tracing::debug!(?self.peer_url, "peer closed");

        if let Some(ep_inner) = self.ep.upgrade() {
            ep_inner.lock().unwrap().drop_peer_url(&self.peer_url);
        }

        let evt_send = self.evt_send.clone();
        let peer_url = self.peer_url.clone();
        tokio::task::spawn(async move {
            let _ = evt_send
                .send(EndpointEvent::Disconnected { peer_url })
                .await;
        });
    }
}

/// The main event-loop task for a connection.
///
/// This function is responsible for managing the connection lifecycle, including sending and
/// receiving messages, handling preflight checks, and notifying the endpoint of connection
/// events (connected, disconnected).
///
/// Crucially, the function is also responsible for closing the connection using the [`DropPeer`]
/// guard when the task is dropped or exits. This means that the task *must not* stall or block
/// indefinitely, as that would leave a dead connection in state that the endpoint doesn't know
/// it should replace. That is the reason for the `tokio::time::timeout` calls in this function,
/// and any new code added here should also respect that requirement to avoid stalling the task.
#[allow(clippy::too_many_arguments)]
async fn task(
    config: Arc<Config>,
    recv_limit: Arc<tokio::sync::Semaphore>,
    ep: Weak<Mutex<EpInner>>,
    conn: Option<DynBackWaitCon>,
    peer_url: PeerUrl,
    evt_send: tokio::sync::mpsc::Sender<EndpointEvent>,
    ready: Arc<Mutex<MaybeReady>>,
) {
    // establish our cleanup drop guard
    let _drop = DropPeer {
        ep,
        peer_url: peer_url.clone(),
        evt_send: evt_send.clone(),
    };

    let mut wc = match conn {
        None => return,
        Some(wc) => wc,
    };

    // wait for the connection to actually be established
    let (conn, mut conn_recv) = match wc.wait(config.timeout, recv_limit).await
    {
        Ok((conn, conn_recv)) => (conn, conn_recv),
        Err(err) => {
            tracing::debug!(?err, ?peer_url, "connection wait error");
            return;
        }
    };

    // manage preflight if configured to do so
    if let Some((pf_send, pf_check)) = &config.preflight {
        match tokio::time::timeout(config.timeout, async {
            let pf_data = match pf_send(&peer_url).await {
                Ok(pf_data) => pf_data,
                Err(err) => {
                    tracing::debug!(
                        ?err,
                        ?peer_url,
                        "preflight get send error"
                    );
                    return Err(());
                }
            };

            if let Err(err) = conn.send(pf_data).await {
                tracing::debug!(?err, ?peer_url, "preflight send error");
                return Err(());
            }

            let pf_data = match conn_recv.recv().await {
                Some(pf_data) => pf_data,
                None => {
                    tracing::debug!(
                        ?peer_url,
                        "closed awaiting preflight data"
                    );
                    return Err(());
                }
            };

            if let Err(err) = pf_check(&peer_url, pf_data).await {
                tracing::debug!(?err, ?peer_url, "preflight check error");
                return Err(());
            }

            Ok(())
        })
        .await
        {
            Ok(Ok(())) => {
                tracing::debug!(?peer_url, "preflight check passed");
            }
            Ok(Err(_)) => {
                // Error already logged in the preflight check
                return;
            }
            Err(err) => {
                tracing::info!(?err, ?peer_url, "preflight timed out");
                return;
            }
        }
    }

    // store a handle for use sending outgoing data
    {
        let mut lock = ready.lock().unwrap();
        if let MaybeReady::Wait(w) = &*lock {
            w.close();
        }
        *lock = MaybeReady::Ready(Arc::new(conn));
    }

    // send the notification that the connection is ready
    drop(ready);

    // send an event saying there is a new connection
    match tokio::time::timeout(
        std::time::Duration::from_secs(10),
        evt_send.send(EndpointEvent::Connected {
            peer_url: peer_url.clone(),
        }),
    )
    .await
    {
        Ok(Ok(())) => {}
        Ok(Err(_)) => {
            tracing::debug!(?peer_url, "failed to send connected event");
        }
        Err(_) => {
            tracing::debug!(?peer_url, "timed out sending connected event");
            // Note that a failure to notify the endpoint of a new connection isn't really fatal
            // because the connection is still established and valid. It should just mean that the
            // application using this library isn't consuming events fast enough.
        }
    }

    tracing::info!(?peer_url, "peer connected");

    // main event loop handling incoming messages
    //
    // It is acceptable for this loop to block the task while waiting for an incoming message. If
    // the application isn't using the connection or isn't getting responses from the remote peer
    // then it is free to close the connection. Here, we don't know whether the connection is
    // in use or not, so we'll just wait for messages.
    while let Some(msg) = conn_recv.recv().await {
        let _ = evt_send
            .send(EndpointEvent::Message {
                peer_url: peer_url.clone(),
                message: msg,
            })
            .await;
    }

    // the above loop ended, notify of disconnect
    match tokio::time::timeout(
        std::time::Duration::from_secs(10),
        evt_send.send(EndpointEvent::Disconnected {
            peer_url: peer_url.clone(),
        }),
    )
    .await
    {
        Ok(Ok(())) => {}
        Ok(Err(_)) => {
            tracing::debug!(?peer_url, "failed to send disconnected event");
        }
        Err(_) => {
            tracing::debug!(?peer_url, "timed out sending disconnected event");
        }
    }

    // all other cleanup is handled by the drop guard
}
