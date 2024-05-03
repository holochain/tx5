use crate::*;

use tx5_connection::tx5_signal::*;
use tx5_connection::*;

enum MaybeReady {
    Ready(Arc<Hub>),
    Wait(Arc<tokio::sync::Semaphore>),
}

pub(crate) struct Sig {
    pub(crate) listener: bool,
    pub(crate) sig_url: SigUrl,
    ready: Arc<Mutex<MaybeReady>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Sig {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Sig {
    pub fn new(
        ep: Weak<Mutex<EpInner>>,
        config: Arc<Config>,
        sig_url: SigUrl,
        listener: bool,
        evt_send: tokio::sync::mpsc::Sender<EndpointEvent>,
    ) -> Arc<Self> {
        Arc::new_cyclic(|this| {
            let wait = Arc::new(tokio::sync::Semaphore::new(0));
            let ready = Arc::new(Mutex::new(MaybeReady::Wait(wait)));

            let task = tokio::task::spawn(task(
                ep,
                this.clone(),
                config,
                sig_url.clone(),
                listener,
                evt_send,
                ready.clone(),
            ));

            Self {
                listener,
                sig_url,
                ready,
                task,
            }
        })
    }

    pub async fn ready(&self) {
        let w = match &*self.ready.lock().unwrap() {
            MaybeReady::Ready(_) => return,
            MaybeReady::Wait(w) => w.clone(),
        };

        let _ = w.acquire().await;
    }

    pub async fn connect(
        &self,
        pub_key: PubKey,
    ) -> Result<(Arc<Conn>, ConnRecv)> {
        let hub = match &*self.ready.lock().unwrap() {
            MaybeReady::Ready(h) => h.clone(),
            _ => return Err(Error::other("not ready")),
        };
        hub.connect(pub_key).await
    }
}

async fn connect_loop(
    config: Arc<Config>,
    sig_url: SigUrl,
    listener: bool,
) -> (Hub, HubRecv) {
    tracing::trace!(?config, ?sig_url, ?listener, "signal try connect");

    let mut wait = config.backoff_start;

    let signal_config = Arc::new(SignalConfig {
        listener,
        allow_plain_text: config.signal_allow_plain_text,
        ..Default::default()
    });

    loop {
        match tokio::time::timeout(
            config.timeout,
            Hub::new(&sig_url, signal_config.clone()),
        )
        .await
        .map_err(Error::other)
        {
            Ok(Ok(r)) => return r,
            Err(err) | Ok(Err(err)) => {
                tracing::debug!(?err, "signal connect error")
            }
        }

        wait *= 2;
        if wait > config.backoff_max {
            wait = config.backoff_max;
        }
        tokio::time::sleep(wait).await;
    }
}

struct DropSig {
    ep: Weak<Mutex<EpInner>>,
    sig: Weak<Sig>,
}

impl Drop for DropSig {
    fn drop(&mut self) {
        if let Some(ep_inner) = self.ep.upgrade() {
            if let Some(sig) = self.sig.upgrade() {
                ep_inner.lock().unwrap().drop_sig(sig);
            }
        }
    }
}

async fn task(
    ep: Weak<Mutex<EpInner>>,
    this: Weak<Sig>,
    config: Arc<Config>,
    sig_url: SigUrl,
    listener: bool,
    evt_send: tokio::sync::mpsc::Sender<EndpointEvent>,
    ready: Arc<Mutex<MaybeReady>>,
) {
    let _drop = DropSig {
        ep: ep.clone(),
        sig: this,
    };

    let (hub, mut hub_recv) =
        connect_loop(config.clone(), sig_url.clone(), listener).await;

    let local_url = sig_url.to_peer(hub.pub_key().clone());

    let hub = Arc::new(hub);

    {
        let mut lock = ready.lock().unwrap();
        if let MaybeReady::Wait(w) = &*lock {
            w.close();
        }
        *lock = MaybeReady::Ready(hub);
    }

    drop(ready);

    let _ = evt_send
        .send(EndpointEvent::ListeningAddressOpen {
            local_url: local_url.clone(),
        })
        .await;

    tracing::info!(?local_url, "signal connected");

    while let Some((conn, conn_recv)) = hub_recv.accept().await {
        if let Some(ep) = ep.upgrade() {
            let peer_url = sig_url.to_peer(conn.pub_key().clone());
            ep.lock().unwrap().accept_peer(peer_url, conn, conn_recv);
        }
    }

    tracing::trace!(?local_url, "signal closing");

    // wait at the end to account for a delay before the next try
    tokio::time::sleep(config.backoff_start).await;

    let _ = evt_send
        .send(EndpointEvent::ListeningAddressClosed {
            local_url: local_url.clone(),
        })
        .await;

    tracing::debug!(?local_url, "signal closed");
}
