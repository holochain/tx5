use crate::*;

use tx5_connection::tx5_signal::*;
use tx5_connection::*;

enum MaybeReady {
    Ready(Arc<Tx5ConnectionHub>),
    Wait(Arc<tokio::sync::Semaphore>),
}

pub(crate) struct Sig {
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
                sig_url,
                listener,
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

    pub async fn connect(&self, pub_key: PubKey) -> Result<Arc<Tx5Connection>> {
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
) -> Tx5ConnectionHub {
    let mut wait = config.backoff_start;

    let signal_config = Arc::new(SignalConfig {
        listener,
        allow_plain_text: config.signal_allow_plain_text,
        ..Default::default()
    });

    loop {
        if let Ok(sig) =
            SignalConnection::connect(&sig_url, signal_config.clone()).await
        {
            return Tx5ConnectionHub::new(sig);
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

    let hub = connect_loop(config.clone(), sig_url.clone(), listener).await;

    let local_url = sig_url.to_peer(hub.pub_key().clone());

    let hub = Arc::new(hub);
    let weak_hub = Arc::downgrade(&hub);

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

    while let Some(hub) = weak_hub.upgrade() {
        if let Some(conn) = hub.accept().await {
            if let Some(ep) = ep.upgrade() {
                let peer_url = sig_url.to_peer(conn.pub_key().clone());
                ep.lock().unwrap().insert_peer(peer_url, conn);
            }
        } else {
            break;
        }
    }

    // wait at the end to account for a delay before the next try
    tokio::time::sleep(config.backoff_start).await;

    let _ = evt_send
        .send(EndpointEvent::ListeningAddressClosed { local_url })
        .await;
}
