use crate::*;

enum MaybeReady {
    Ready(DynBackEp),
    Wait(Arc<tokio::sync::Semaphore>),
}

/// Manage a backend signal connection. This is actually represented
/// by the tx5-connection "Hub" type.
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
        resp_url: Option<tokio::sync::oneshot::Sender<PeerUrl>>,
    ) -> Arc<Self> {
        Arc::new_cyclic(|this| {
            let wait = Arc::new(tokio::sync::Semaphore::new(0));
            let ready = Arc::new(Mutex::new(MaybeReady::Wait(wait)));

            // spawn the main event-loop for the signal connection
            let task = tokio::task::spawn(task(
                ep,
                this.clone(),
                config,
                sig_url.clone(),
                listener,
                evt_send,
                ready.clone(),
                resp_url,
            ));

            Self {
                listener,
                sig_url,
                ready,
                task,
            }
        })
    }

    /// This future resolves when the signal connection is ready to be used.
    pub async fn ready(&self) {
        let w = match &*self.ready.lock().unwrap() {
            MaybeReady::Ready(_) => return,
            MaybeReady::Wait(w) => w.clone(),
        };

        let _ = w.acquire().await;
    }

    /// Get the address at which this peer will be reachable.
    pub fn get_peer_url(&self) -> Option<PeerUrl> {
        match &*self.ready.lock().unwrap() {
            MaybeReady::Wait(_) => None,
            MaybeReady::Ready(hub) => {
                Some(self.sig_url.to_peer(hub.pub_key().clone()))
            }
        }
    }

    /// Attempt to establish a connection to a remote peer.
    pub async fn connect(&self, pub_key: PubKey) -> Result<DynBackWaitCon> {
        let ep = match &*self.ready.lock().unwrap() {
            MaybeReady::Ready(h) => h.clone(),
            _ => return Err(Error::other("not ready")),
        };
        ep.connect(pub_key).await
    }
}

/// Loop attempting to establish a signal connection.
async fn connect_loop(
    config: Arc<Config>,
    sig_url: SigUrl,
    listener: bool,
    mut resp_url: Option<tokio::sync::oneshot::Sender<PeerUrl>>,
) -> (DynBackEp, DynBackEpRecvCon) {
    tracing::debug!(
        target: "NETAUDIT",
        ?config,
        ?sig_url,
        ?listener,
        m = "tx5",
        t = "signal",
        a = "try_connect",
    );

    let mut wait = config.backoff_start;

    loop {
        match tokio::time::timeout(
            config.timeout,
            config.backend_module.connect(&sig_url, listener, &config),
        )
        .await
        .map_err(Error::other)
        {
            Ok(Ok(r)) => {
                if let Some(resp_url) = resp_url.take() {
                    let _ =
                        resp_url.send(sig_url.to_peer(r.0.pub_key().clone()));
                }
                return r;
            }
            Err(err) | Ok(Err(err)) => {
                // drop the response so we can proceed without a peer_url
                let _ = resp_url.take();
                tracing::debug!(
                    target: "NETAUDIT",
                    ?err,
                    m = "tx5",
                    t = "signal",
                    a = "connect_error",
                );
            }
        }

        wait *= 2;
        if wait > config.backoff_max {
            wait = config.backoff_max;
        }
        tokio::time::sleep(wait).await;
    }
}

/// Helper guard for performing cleanup if the signal event loop task closes.
struct DropSig {
    inner: Weak<Mutex<EpInner>>,
    sig_url: SigUrl,
    local_url: Option<PeerUrl>,
    sig: Weak<Sig>,
}

impl Drop for DropSig {
    fn drop(&mut self) {
        tracing::debug!(
            target: "NETAUDIT",
            sig_url = ?self.sig_url,
            local_url = ?self.local_url,
            m = "tx5",
            t = "signal",
            a = "drop",
        );

        if let Some(inner) = self.inner.upgrade() {
            if let Some(sig) = self.sig.upgrade() {
                inner.lock().unwrap().drop_sig(sig);
            }
        }
    }
}

/// This is the main event-loop task for the signal connection
#[allow(clippy::too_many_arguments)]
async fn task(
    inner: Weak<Mutex<EpInner>>,
    this: Weak<Sig>,
    config: Arc<Config>,
    sig_url: SigUrl,
    listener: bool,
    evt_send: tokio::sync::mpsc::Sender<EndpointEvent>,
    ready: Arc<Mutex<MaybeReady>>,
    resp_url: Option<tokio::sync::oneshot::Sender<PeerUrl>>,
) {
    // establish our drop-guard incase this task is aborted or exits
    let mut drop_g = DropSig {
        inner: inner.clone(),
        sig_url: sig_url.clone(),
        local_url: None,
        sig: this,
    };

    // create a connection to the signal server (tx5-connection "Hub").
    let (ep, mut ep_recv) =
        connect_loop(config.clone(), sig_url.clone(), listener, resp_url).await;

    // get our url
    let local_url = sig_url.to_peer(ep.pub_key().clone());
    drop_g.local_url = Some(local_url.clone());

    // mark ourselves as ready and store a handle
    // that will be used for establishing outgoing connections
    {
        let mut lock = ready.lock().unwrap();
        if let MaybeReady::Wait(w) = &*lock {
            w.close();
        }
        *lock = MaybeReady::Ready(ep);
    }

    // now notify that we are ready
    drop(ready);

    if listener {
        // if we are a listener, emit the listening event
        let _ = evt_send
            .send(EndpointEvent::ListeningAddressOpen {
                local_url: local_url.clone(),
            })
            .await;
    }

    tracing::debug!(
        target: "NETAUDIT",
        ?local_url,
        m = "tx5",
        t = "signal",
        a = "connected",
    );

    // now, our only task is to listen for incoming connections
    // and process them
    while let Some(wc) = ep_recv.recv().await {
        if let Some(inner) = inner.upgrade() {
            let peer_url = sig_url.to_peer(wc.pub_key().clone());
            inner.lock().unwrap().accept_peer(peer_url, wc);
        }
    }

    tracing::debug!(
        target: "NETAUDIT",
        ?local_url,
        m = "tx5",
        t = "signal",
        a = "closing",
    );

    // wait at the end to account for a delay before the next try
    tokio::time::sleep(config.backoff_start).await;

    if listener {
        // we are closing. If we are a listener, notify the fact
        let _ = evt_send
            .send(EndpointEvent::ListeningAddressClosed {
                local_url: local_url.clone(),
            })
            .await;
    }

    tracing::debug!(
        target: "NETAUDIT",
        ?local_url,
        m = "tx5",
        t = "signal",
        a = "closed",
    );

    // all other cleanup is handled by the drop guard
}
