use crate::*;

use tx5_connection::*;

enum MaybeReady {
    Ready(Arc<Tx5ConnFramed>),
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
        conn: Arc<Tx5Connection>,
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
                conn,
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
    let conn = connect_loop(config.clone(), ep.clone(), peer_url.clone()).await;

    task(
        config, recv_limit, ep, this, conn, peer_url, evt_send, ready,
    )
    .await;
}

async fn connect_loop(
    config: Arc<Config>,
    ep: Weak<Mutex<EpInner>>,
    peer_url: PeerUrl,
) -> Arc<Tx5Connection> {
    let mut wait = config.backoff_start;

    loop {
        if let Some(ep) = ep.upgrade() {
            let sig = ep.lock().unwrap().assert_sig(peer_url.to_sig(), false);
            sig.ready().await;
            if let Ok(conn) = sig.connect(peer_url.pub_key().clone()).await {
                return conn;
            }
        }

        wait *= 2;
        if wait > config.backoff_max {
            wait = config.backoff_max;
        }
        tokio::time::sleep(wait).await;
    }
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
    conn: Arc<Tx5Connection>,
    peer_url: PeerUrl,
    evt_send: tokio::sync::mpsc::Sender<EndpointEvent>,
    ready: Arc<Mutex<MaybeReady>>,
) {
    let _drop = DropPeer { ep, peer: this };

    conn.ready().await;

    let conn = match Tx5ConnFramed::new(conn, recv_limit).await {
        Ok(conn) => Arc::new(conn),
        Err(_) => return,
    };

    let weak_conn = Arc::downgrade(&conn);

    {
        let mut lock = ready.lock().unwrap();
        if let MaybeReady::Wait(w) = &*lock {
            w.close();
        }
        *lock = MaybeReady::Ready(conn);
    }

    drop(ready);

    let _ = evt_send
        .send(EndpointEvent::Connected {
            peer_url: peer_url.clone(),
        })
        .await;

    while let Some(conn) = weak_conn.upgrade() {
        if let Some(msg) = conn.recv().await {
            let _ = evt_send
                .send(EndpointEvent::Message {
                    peer_url: peer_url.clone(),
                    message: msg,
                })
                .await;
        } else {
            break;
        }
    }

    // wait at the end to account for a delay before the next try
    tokio::time::sleep(config.backoff_start).await;

    let _ = evt_send
        .send(EndpointEvent::Disconnected { peer_url })
        .await;
}
