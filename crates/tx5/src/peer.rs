use crate::*;

use tx5_connection::*;

enum MaybeReady {
    Ready(Arc<FramedConn>),
    Wait(Arc<tokio::sync::Semaphore>),
}

pub(crate) struct Peer {
    ready: Arc<Mutex<MaybeReady>>,
    task: tokio::task::JoinHandle<()>,
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
        Arc::new_cyclic(|this| {
            let wait = Arc::new(tokio::sync::Semaphore::new(0));
            let ready = Arc::new(Mutex::new(MaybeReady::Wait(wait)));

            let task = tokio::task::spawn(connect(
                config,
                recv_limit,
                ep,
                this.clone(),
                peer_url,
                evt_send,
                ready.clone(),
            ));

            Self { ready, task }
        })
    }

    pub fn new_accept(
        config: Arc<Config>,
        recv_limit: Arc<tokio::sync::Semaphore>,
        ep: Weak<Mutex<EpInner>>,
        peer_url: PeerUrl,
        conn: Arc<Conn>,
        conn_recv: ConnRecv,
        evt_send: tokio::sync::mpsc::Sender<EndpointEvent>,
    ) -> Arc<Self> {
        Arc::new_cyclic(|this| {
            let wait = Arc::new(tokio::sync::Semaphore::new(0));
            let ready = Arc::new(Mutex::new(MaybeReady::Wait(wait)));

            let task = tokio::task::spawn(task(
                config,
                recv_limit,
                ep,
                this.clone(),
                Some((conn, conn_recv)),
                peer_url,
                evt_send,
                ready.clone(),
            ));

            Self { ready, task }
        })
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
    this: Weak<Peer>,
    peer_url: PeerUrl,
    evt_send: tokio::sync::mpsc::Sender<EndpointEvent>,
    ready: Arc<Mutex<MaybeReady>>,
) {
    tracing::trace!(?peer_url, "peer try connect");

    let conn = if let Some(ep) = ep.upgrade() {
        match tokio::time::timeout(config.timeout, async {
            let sig = ep.lock().unwrap().assert_sig(peer_url.to_sig(), false);
            sig.ready().await;
            sig.connect(peer_url.pub_key().clone()).await
        })
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

    task(
        config, recv_limit, ep, this, conn, peer_url, evt_send, ready,
    )
    .await;
}

struct DropPeer {
    ep: Weak<Mutex<EpInner>>,
    peer: Weak<Peer>,
}

impl Drop for DropPeer {
    fn drop(&mut self) {
        if let Some(ep_inner) = self.ep.upgrade() {
            if let Some(peer) = self.peer.upgrade() {
                ep_inner.lock().unwrap().drop_peer(peer);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn task(
    config: Arc<Config>,
    recv_limit: Arc<tokio::sync::Semaphore>,
    ep: Weak<Mutex<EpInner>>,
    this: Weak<Peer>,
    conn: Option<(Arc<Conn>, ConnRecv)>,
    peer_url: PeerUrl,
    evt_send: tokio::sync::mpsc::Sender<EndpointEvent>,
    ready: Arc<Mutex<MaybeReady>>,
) {
    let _drop = DropPeer { ep, peer: this };

    let (conn, conn_recv) = match conn {
        None => return,
        Some(conn) => conn,
    };

    conn.ready().await;

    let (conn, mut conn_recv) =
        match FramedConn::new(conn, conn_recv, recv_limit).await {
            Ok(conn) => conn,
            Err(_) => return,
        };

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

    // wait at the end to account for a delay before the next try
    tokio::time::sleep(config.backoff_start).await;

    let _ = evt_send
        .send(EndpointEvent::Disconnected {
            peer_url: peer_url.clone(),
        })
        .await;

    tracing::debug!(?peer_url, "peer closed");
}
