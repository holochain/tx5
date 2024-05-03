use crate::*;

/// Endpoint event.
pub enum EndpointEvent {
    /// We have a new listening address at which we can be contacted.
    ListeningAddressOpen {
        /// An address at which we can be reached.
        local_url: PeerUrl,
    },

    /// A listening address at which we were reachable has been closed.
    ListeningAddressClosed {
        /// The address at which we can no longer be reached.
        local_url: PeerUrl,
    },

    /// Connection established.
    Connected {
        /// Url of the remote peer.
        peer_url: PeerUrl,
    },

    /// Connection closed.
    Disconnected {
        /// Url of the remote peer.
        peer_url: PeerUrl,
    },

    /// Receiving an incoming message from a remote peer.
    Message {
        /// Url of the remote peer.
        peer_url: PeerUrl,

        /// Message sent by the remote peer.
        message: Vec<u8>,
    },
}

impl std::fmt::Debug for EndpointEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ListeningAddressOpen { local_url } => f
                .debug_struct("ListeningAddressOpen")
                .field("peer_url", local_url)
                .finish(),
            Self::ListeningAddressClosed { local_url } => f
                .debug_struct("ListeningAddressClosed")
                .field("peer_url", local_url)
                .finish(),
            Self::Connected { peer_url } => f
                .debug_struct("Connected")
                .field("peer_url", peer_url)
                .finish(),
            Self::Disconnected { peer_url } => f
                .debug_struct("Disconnected")
                .field("peer_url", peer_url)
                .finish(),
            Self::Message { peer_url, .. } => f
                .debug_struct("Message")
                .field("peer_url", peer_url)
                .finish(),
        }
    }
}

pub(crate) struct EpInner {
    this: Weak<Mutex<EpInner>>,
    config: Arc<Config>,
    recv_limit: Arc<tokio::sync::Semaphore>,
    evt_send: tokio::sync::mpsc::Sender<EndpointEvent>,
    sig_map: HashMap<SigUrl, Arc<Sig>>,
    peer_map: HashMap<PeerUrl, Arc<Peer>>,
}

impl EpInner {
    pub fn drop_sig(&mut self, sig: Arc<Sig>) {
        let sig_url = sig.sig_url.clone();
        let listener = sig.listener;

        let should_remove = match self.sig_map.get(&sig_url) {
            Some(s) => Arc::ptr_eq(s, &sig),
            None => false,
        };

        if should_remove {
            self.sig_map.remove(&sig_url);

            if listener {
                self.assert_sig(sig_url, listener);
            }
        }
    }

    pub fn assert_sig(&mut self, sig_url: SigUrl, listener: bool) -> Arc<Sig> {
        self.sig_map
            .entry(sig_url.clone())
            .or_insert_with(|| {
                Sig::new(
                    self.this.clone(),
                    self.config.clone(),
                    sig_url,
                    listener,
                    self.evt_send.clone(),
                )
            })
            .clone()
    }

    pub fn drop_peer(&mut self, peer: Arc<Peer>) {
        self.peer_map.retain(|_, p| !Arc::ptr_eq(p, &peer))
    }

    pub fn connect_peer(&mut self, peer_url: PeerUrl) -> Arc<Peer> {
        if let Some(peer) = self.peer_map.get(&peer_url) {
            return peer.clone();
        }

        self.peer_map
            .entry(peer_url.clone())
            .or_insert_with(|| {
                Peer::new_connect(
                    self.config.clone(),
                    self.recv_limit.clone(),
                    self.this.clone(),
                    peer_url,
                    self.evt_send.clone(),
                )
            })
            .clone()
    }

    pub fn accept_peer(
        &mut self,
        peer_url: PeerUrl,
        conn: Arc<tx5_connection::Conn>,
        conn_recv: tx5_connection::ConnRecv,
    ) {
        self.peer_map.entry(peer_url.clone()).or_insert_with(|| {
            Peer::new_accept(
                self.config.clone(),
                self.recv_limit.clone(),
                self.this.clone(),
                peer_url,
                conn,
                conn_recv,
                self.evt_send.clone(),
            )
        });
    }

    pub fn clone_peers(&mut self) -> Vec<Arc<Peer>> {
        self.peer_map.values().cloned().collect()
    }
}

/// Tx5 endpoint.
pub struct Endpoint {
    config: Arc<Config>,
    inner: Arc<Mutex<EpInner>>,
    evt_recv: tokio::sync::Mutex<tokio::sync::mpsc::Receiver<EndpointEvent>>,
}

impl Endpoint {
    /// Construct a new tx5 endpoint.
    pub fn new(config: Arc<Config>) -> Self {
        let recv_limit = Arc::new(tokio::sync::Semaphore::new(
            config.incoming_message_bytes_max as usize,
        ));

        let (evt_send, evt_recv) = tokio::sync::mpsc::channel(32);

        Self {
            config: config.clone(),
            inner: Arc::new_cyclic(|this| {
                Mutex::new(EpInner {
                    this: this.clone(),
                    config,
                    recv_limit,
                    evt_send,
                    sig_map: HashMap::default(),
                    peer_map: HashMap::default(),
                })
            }),
            evt_recv: tokio::sync::Mutex::new(evt_recv),
        }
    }

    /// Receive an event from this tx5 endpoint.
    pub async fn recv(&self) -> Option<EndpointEvent> {
        self.evt_recv.lock().await.recv().await
    }

    /// Connect to a signal server as a listener, allowing incoming connections.
    /// You probably only want to call this once.
    pub fn listen(&self, sig_url: SigUrl) {
        let _ = self.inner.lock().unwrap().assert_sig(sig_url, true);
    }

    /// Send data to a remote on this tx5 endpoint.
    /// The future returned from this method will resolve when
    /// the data is handed off to our networking backend.
    pub async fn send(&self, peer_url: PeerUrl, data: Vec<u8>) -> Result<()> {
        tokio::time::timeout(self.config.timeout, async {
            let peer = self.inner.lock().unwrap().connect_peer(peer_url);
            peer.ready().await;
            peer.send(data).await
        })
        .await?
    }

    /// Broadcast data to all connections that happen to be open.
    /// If no connections are open, no data will be broadcast.
    /// The future returned from this method will resolve when all
    /// broadcast messages have been handed off to our networking backend
    /// (or have timed out).
    pub async fn broadcast(&self, data: &[u8]) {
        let peers = self.inner.lock().unwrap().clone_peers();

        let timeout = self.config.timeout;

        let all = peers
            .into_iter()
            .map(|peer| {
                let data = data.to_vec();
                tokio::task::spawn(async move {
                    let _ = tokio::time::timeout(timeout, async {
                        peer.ready().await;
                        let _ = peer.send(data).await;
                    })
                    .await;
                })
            })
            .collect::<Vec<_>>();

        for task in all {
            let _ = task.await;
        }
    }
}
