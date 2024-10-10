use crate::*;

use tx5_connection::*;

enum MaybeReady {
    Ready(Arc<DynBackCon>),
    Wait(Arc<tokio::sync::Semaphore>),
}

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

            Self {
                ready,
                task,
                pub_key,
                opened_at_s,
            }
        })
    }

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

    pub fn is_using_webrtc(&self) -> bool {
        if let MaybeReady::Ready(r) = &*self.ready.lock().unwrap() {
            r.is_using_webrtc()
        } else {
            false
        }
    }

    pub fn get_stats(&self) -> ConnStats {
        if let MaybeReady::Ready(r) = &*self.ready.lock().unwrap() {
            r.get_stats()
        } else {
            ConnStats::default()
        }
    }

    pub async fn ready(&self) {
        let w = match &*self.ready.lock().unwrap() {
            MaybeReady::Ready(_) => return,
            MaybeReady::Wait(w) => w.clone(),
        };

        let _ = w.acquire().await;
    }

    pub async fn send(&self, msg: Vec<u8>) -> Result<()> {
        let conn = match &*self.ready.lock().unwrap() {
            MaybeReady::Ready(c) => c.clone(),
            _ => return Err(Error::other("not ready")),
        };
        conn.send(msg).await
    }
}

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

        match tokio::time::timeout(config.timeout, connect_fut)
            .await
            .map_err(Error::other)
        {
            Ok(Ok(conn)) => Some(conn),
            Err(err) | Ok(Err(err)) => {
                tracing::debug!(?err, "peer connect error");
                None
            }
        }
    } else {
        None
    };

    task(config, recv_limit, ep, conn, peer_url, evt_send, ready).await;
}

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
    let _drop = DropPeer {
        ep,
        peer_url: peer_url.clone(),
        evt_send: evt_send.clone(),
    };

    let mut wc = match conn {
        None => return,
        Some(wc) => wc,
    };

    let (conn, mut conn_recv) = match wc.wait(recv_limit).await {
        Ok((conn, conn_recv)) => (conn, conn_recv),
        Err(err) => {
            tracing::debug!(?err, "connection wait error");
            return;
        }
    };

    if let Some((pf_send, pf_check)) = &config.preflight {
        let pf_data = match pf_send(&peer_url).await {
            Ok(pf_data) => pf_data,
            Err(err) => {
                tracing::debug!(?err, "preflight get send error");
                return;
            }
        };

        if let Err(err) = conn.send(pf_data).await {
            tracing::debug!(?err, "preflight send error");
            return;
        }

        let pf_data = match conn_recv.recv().await {
            Some(pf_data) => pf_data,
            None => {
                tracing::debug!("closed awaiting preflight data");
                return;
            }
        };

        if let Err(err) = pf_check(&peer_url, pf_data).await {
            tracing::debug!(?err, "preflight check error");
            return;
        }
    }

    {
        let mut lock = ready.lock().unwrap();
        if let MaybeReady::Wait(w) = &*lock {
            w.close();
        }
        *lock = MaybeReady::Ready(Arc::new(conn));
    }

    drop(ready);

    let _ = evt_send
        .send(EndpointEvent::Connected {
            peer_url: peer_url.clone(),
        })
        .await;

    tracing::info!(?peer_url, "peer connected");

    while let Some(msg) = conn_recv.recv().await {
        let _ = evt_send
            .send(EndpointEvent::Message {
                peer_url: peer_url.clone(),
                message: msg,
            })
            .await;
    }

    let _ = evt_send
        .send(EndpointEvent::Disconnected {
            peer_url: peer_url.clone(),
        })
        .await;
}
