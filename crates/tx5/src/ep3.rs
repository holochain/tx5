//! Module containing tx5 endpoint version 3 types.

use crate::deps::lair_keystore_api;
use crate::deps::sodoken;
use crate::AbortableTimedSharedFuture;
use crate::BackBuf;
use futures::future::BoxFuture;
use lair_keystore_api::prelude::*;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};
use tx5_core::{Error, EventRecv, EventSend, Id, Result, Tx5Url};

fn next_uniq() -> u64 {
    static UNIQ: std::sync::atomic::AtomicU64 =
        std::sync::atomic::AtomicU64::new(1);
    UNIQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

type CRes<T> = std::result::Result<T, Error>;

/// Events generated by a tx5 endpoint version 3.
pub enum Ep3Event {
    /// A fatal error indicating the endpoint is no longer viable.
    Error(Error),

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

        /// Permit counting the bytes allowed in memory on the receive side.
        permit: tokio::sync::OwnedSemaphorePermit,
    },
}

impl From<Error> for Ep3Event {
    fn from(err: Error) -> Self {
        Self::Error(err)
    }
}

impl std::fmt::Debug for Ep3Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Error(err) => {
                f.debug_struct("Error").field("err", err).finish()
            }
            Self::Connected { peer_url } => {
                let url = format!("{peer_url}");
                f.debug_struct("Connected").field("peer_url", &url).finish()
            }
            Self::Disconnected { peer_url } => {
                let url = format!("{peer_url}");
                f.debug_struct("Disconnected")
                    .field("peer_url", &url)
                    .finish()
            }
            Self::Message { peer_url, .. } => {
                let url = format!("{peer_url}");
                f.debug_struct("Message").field("peer_url", &url).finish()
            }
        }
    }
}

/// A signal server url.
pub type SigUrl = Tx5Url;

/// A peer connection url.
pub type PeerUrl = Tx5Url;

type SigMap = HashMap<SigUrl, (u64, AbortableTimedSharedFuture<Arc<Sig>>)>;

/// Callback in charge of sending preflight data if any.
pub type PreflightSendCb = Arc<
    dyn Fn(&PeerUrl) -> BoxFuture<'static, Result<Vec<u8>>>
        + 'static
        + Send
        + Sync,
>;

/// Callback in charge of validating preflight data if any.
pub type PreflightCheckCb = Arc<
    dyn Fn(&PeerUrl, Vec<u8>) -> BoxFuture<'static, Result<()>>
        + 'static
        + Send
        + Sync,
>;

/// Tx5 endpoint version 3 configuration.
pub struct Config3 {
    /// Maximum count of open connections. Default 4096.
    pub connection_count_max: u32,

    /// Max backend send buffer bytes (per connection). Default 64 KiB.
    pub send_buffer_bytes_max: u32,

    /// Max backend recv buffer bytes (per connection). Default 64 KiB.
    pub recv_buffer_bytes_max: u32,

    /// Maximum receive message reconstruction bytes in memory
    /// (accross entire endpoint). Default 512 MiB.
    pub incoming_message_bytes_max: u32,

    /// Maximum size of an individual message. Default 16 MiB.
    pub message_size_max: u32,

    /// Default timeout for network operations. Default 60 seconds.
    pub timeout: std::time::Duration,

    /// Starting backoff duration for retries. Default 5 seconds.
    pub backoff_start: std::time::Duration,

    /// Max backoff duration for retries. Default 60 seconds.
    pub backoff_max: std::time::Duration,

    /// If the protocol should manage a preflight message,
    /// set the callbacks here, otherwise no preflight will
    /// be sent nor validated. Default: None.
    pub preflight: Option<(PreflightSendCb, PreflightCheckCb)>,
}

impl Default for Config3 {
    fn default() -> Self {
        Self {
            connection_count_max: 4096,
            send_buffer_bytes_max: 64 * 1024,
            recv_buffer_bytes_max: 64 * 1024,
            incoming_message_bytes_max: 512 * 1024 * 1024,
            message_size_max: 16 * 1024 * 1024,
            timeout: std::time::Duration::from_secs(60),
            backoff_start: std::time::Duration::from_secs(5),
            backoff_max: std::time::Duration::from_secs(60),
            preflight: None,
        }
    }
}

#[derive(Default)]
struct BanMap(HashMap<Id, tokio::time::Instant>);

impl BanMap {
    fn set_ban(&mut self, id: Id, until: tokio::time::Instant) {
        self.0.insert(id, until);
    }

    fn is_banned(&mut self, id: Id) -> bool {
        let now = tokio::time::Instant::now();
        if let Some(until) = self.0.get(&id).cloned() {
            if now < until {
                true
            } else {
                self.0.remove(&id);
                false
            }
        } else {
            false
        }
    }
}

pub(crate) struct EpShared {
    config: Arc<Config3>,
    this_id: Id,
    ep_uniq: u64,
    lair_tag: Arc<str>,
    lair_client: LairClient,
    sig_limit: Arc<tokio::sync::Semaphore>,
    peer_limit: Arc<tokio::sync::Semaphore>,
    recv_recon_limit: Arc<tokio::sync::Semaphore>,
    weak_sig_map: Weak<Mutex<SigMap>>,
    evt_send: EventSend<Ep3Event>,
    ban_map: Mutex<BanMap>,
    metric_conn_count:
        influxive_otel_atomic_obs::AtomicObservableUpDownCounterI64,
}

/// Tx5 endpoint version 3.
pub struct Ep3 {
    ep: Arc<EpShared>,
    _lair_keystore: lair_keystore_api::in_proc_keystore::InProcKeystore,
    _sig_map: Arc<Mutex<SigMap>>,
    listen_sigs: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
}

impl std::fmt::Debug for Ep3 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ep3")
            .field("this_id", &self.ep.this_id)
            .field("ep_uniq", &self.ep.ep_uniq)
            .finish()
    }
}

impl Drop for Ep3 {
    fn drop(&mut self) {
        let handles = std::mem::take(&mut *self.listen_sigs.lock().unwrap());
        for handle in handles {
            handle.abort();
        }
    }
}

impl Ep3 {
    /// Construct a new tx5 endpoint version 3.
    pub async fn new(config: Arc<Config3>) -> (Self, EventRecv<Ep3Event>) {
        use influxive_otel_atomic_obs::MeterExt;
        use opentelemetry_api::metrics::MeterProvider;

        let sig_limit = Arc::new(tokio::sync::Semaphore::new(
            config.connection_count_max as usize,
        ));

        let peer_limit = Arc::new(tokio::sync::Semaphore::new(
            config.connection_count_max as usize,
        ));

        let recv_recon_limit = Arc::new(tokio::sync::Semaphore::new(
            config.incoming_message_bytes_max as usize,
        ));

        let lair_tag: Arc<str> =
            rand_utf8::rand_utf8(&mut rand::thread_rng(), 32).into();

        let passphrase = sodoken::BufRead::new_no_lock(
            rand_utf8::rand_utf8(&mut rand::thread_rng(), 32).as_bytes(),
        );

        // this is a memory keystore,
        // so weak persistence security is okay,
        // since it will not be persisted.
        // The private keys will still be mem_locked
        // so they shouldn't be swapped to disk.
        let keystore_config = PwHashLimits::Minimum
            .with_exec(|| LairServerConfigInner::new("/", passphrase.clone()))
            .await
            .unwrap();

        let _lair_keystore = PwHashLimits::Minimum
            .with_exec(|| {
                lair_keystore_api::in_proc_keystore::InProcKeystore::new(
                    Arc::new(keystore_config),
                    lair_keystore_api::mem_store::create_mem_store_factory(),
                    passphrase,
                )
            })
            .await
            .unwrap();

        let lair_client = _lair_keystore.new_client().await.unwrap();

        let seed = lair_client
            .new_seed(lair_tag.clone(), None, false)
            .await
            .unwrap();

        let this_id = Id(*seed.x25519_pub_key.0);

        let (evt_send, evt_recv) = EventSend::new(1024);

        let sig_map = Arc::new(Mutex::new(HashMap::new()));
        let weak_sig_map = Arc::downgrade(&sig_map);

        let ep_uniq = next_uniq();

        let meter = opentelemetry_api::global::meter_provider()
            .versioned_meter(
                "tx5",
                None::<&'static str>,
                None::<&'static str>,
                Some(vec![opentelemetry_api::KeyValue::new(
                    "ep_uniq",
                    ep_uniq.to_string(),
                )]),
            );

        let metric_conn_count = meter
            .i64_observable_up_down_counter_atomic("tx5.endpoint.conn.count", 0)
            .with_description("Count of open connections managed by endpoint")
            .init()
            .0;

        let this = Self {
            ep: Arc::new(EpShared {
                config,
                this_id,
                ep_uniq,
                lair_tag,
                lair_client,
                sig_limit,
                peer_limit,
                recv_recon_limit,
                weak_sig_map,
                evt_send,
                ban_map: Mutex::new(BanMap::default()),
                metric_conn_count,
            }),
            _lair_keystore,
            _sig_map: sig_map,
            listen_sigs: Arc::new(Mutex::new(Vec::new())),
        };

        (this, evt_recv)
    }

    /// Establish a listening connection to a signal server,
    /// from which we can accept incoming remote connections.
    /// Returns the client url at which this endpoint may now be addressed.
    pub async fn listen(&self, sig_url: SigUrl) -> Result<PeerUrl> {
        if !sig_url.is_server() {
            return Err(Error::str("Expected SigUrl, got PeerUrl"));
        }

        let ep = self.ep.clone();
        let peer_url = sig_url.to_client(ep.this_id);

        let (wait_send, wait_recv) = tokio::sync::oneshot::channel();
        let mut wait_send = Some(wait_send);

        self.listen_sigs
            .lock()
            .unwrap()
            .push(tokio::task::spawn(async move {
                let mut backoff = ep.config.backoff_start;
                loop {
                    /*
                    tracing::error!(
                        %ep.ep_uniq,
                        %sig_url,
                        "TRY ASSERT SIG",
                    );
                    */
                    match assert_sig(&ep, &sig_url).await {
                        Ok(_) => {
                            //tracing::error!(%ep.ep_uniq, "SIG CONNECTED!");
                            // if the conn is still open it's essentially
                            // a no-op to assert it again, so it's
                            // okay to do that quickly.
                            backoff = ep.config.backoff_start;
                        }
                        Err(_err) => {
                            //tracing::error!(%ep.ep_uniq, ?err, "SIG ERROR!");
                            backoff *= 2;
                            if backoff > ep.config.backoff_max {
                                backoff = ep.config.backoff_max;
                            }
                        }
                    }

                    if let Some(wait_send) = wait_send.take() {
                        let _ = wait_send.send(());
                    }

                    tokio::time::sleep(backoff).await;
                }
            }));

        // await at least one loop of connect attempt before returning
        let _ = wait_recv.await;

        Ok(peer_url)
    }

    /// Close down all connections to, fail all outgoing messages to,
    /// and drop all incoming messages from, the given remote id,
    /// for the specified ban time period.
    pub fn ban(&self, rem_id: Id, span: std::time::Duration) {
        self.ep
            .ban_map
            .lock()
            .unwrap()
            .set_ban(rem_id, tokio::time::Instant::now() + span);

        let fut_list = self
            ._sig_map
            .lock()
            .unwrap()
            .values()
            .map(|v| v.1.clone())
            .collect::<Vec<_>>();
        for fut in fut_list {
            let ep = self.ep.clone();
            // fire and forget
            tokio::task::spawn(async move {
                if let Ok(sig) = fut.await {
                    // see if we are still banning this id.
                    if ep.ban_map.lock().unwrap().is_banned(rem_id) {
                        sig.ban(rem_id);
                    }
                }
            });
        }
    }

    /// Send data to a remote on this tx5 endpoint.
    /// The future returned from this method will resolve when
    /// the data is handed off to our networking backend.
    pub async fn send(&self, peer_url: PeerUrl, data: &[u8]) -> Result<()> {
        if !peer_url.is_client() {
            return Err(Error::str("Expected PeerUrl, got SigUrl"));
        }

        let sig_url = peer_url.to_server();
        let peer_id = peer_url.id().unwrap();

        if self.ep.ban_map.lock().unwrap().is_banned(peer_id) {
            return Err(Error::str("Peer is currently banned"));
        }

        let sig = assert_sig(&self.ep, &sig_url).await?;

        let peer = sig
            .assert_peer(peer_url, peer_id, PeerDir::ActiveOrOutgoing)
            .await?;

        peer.send(data).await
    }

    /// Broadcast data to all connections that happen to be open.
    /// If no connections are open, no data will be broadcast.
    /// The future returned from this method will resolve when all
    /// broadcast messages have been handed off to our networking backend
    /// (or have timed out).
    pub async fn broadcast(&self, data: &[u8]) {
        let mut task_list = Vec::new();

        let fut_list = self
            ._sig_map
            .lock()
            .unwrap()
            .values()
            .map(|v| v.1.clone())
            .collect::<Vec<_>>();

        for fut in fut_list {
            task_list.push(async move {
                // timeouts are built into this future as well
                // as the sig.broadcast function
                if let Ok(sig) = fut.await {
                    sig.broadcast(data).await;
                }
            });
        }

        futures::future::join_all(task_list).await;
    }

    /// Get stats.
    pub async fn get_stats(&self) -> serde_json::Value {
        let mut task_list = Vec::new();

        let mut ban_map = serde_json::Map::new();

        let now = tokio::time::Instant::now();
        for (id, until) in self.ep.ban_map.lock().unwrap().0.iter() {
            ban_map.insert(id.to_string(), (*until - now).as_secs_f64().into());
        }

        let fut_list = self
            ._sig_map
            .lock()
            .unwrap()
            .values()
            .map(|v| v.1.clone())
            .collect::<Vec<_>>();

        for fut in fut_list {
            task_list.push(async move {
                if let Ok(sig) = fut.await {
                    Some(sig.get_stats().await)
                } else {
                    None
                }
            });
        }

        let res: Vec<(Id, serde_json::Value)> =
            futures::future::join_all(task_list)
                .await
                .into_iter()
                .flatten()
                .flatten()
                .collect();

        let mut map = serde_json::Map::default();

        #[cfg(feature = "backend-go-pion")]
        const BACKEND: &str = "go-pion";
        #[cfg(feature = "backend-webrtc-rs")]
        const BACKEND: &str = "webrtc-rs";

        map.insert("backend".into(), BACKEND.into());
        map.insert("thisId".into(), self.ep.this_id.to_string().into());
        map.insert("banned".into(), ban_map.into());

        for (id, v) in res {
            map.insert(id.to_string(), v);
        }

        serde_json::Value::Object(map)
    }
}

async fn assert_sig(ep: &Arc<EpShared>, sig_url: &SigUrl) -> CRes<Arc<Sig>> {
    let sig_map = match ep.weak_sig_map.upgrade() {
        Some(sig_map) => sig_map,
        None => {
            return Err(Error::str(
                "Signal connection failed due to closed endpoint",
            )
            .into())
        }
    };

    let (sig_uniq, fut) = sig_map
        .lock()
        .unwrap()
        .entry(sig_url.clone())
        .or_insert_with(|| {
            let sig_uniq = next_uniq();
            let sig_url = sig_url.clone();
            let ep = ep.clone();
            let _sig_drop = SigDrop {
                ep_uniq: ep.ep_uniq,
                sig_uniq,
                sig_url: sig_url.clone(),
                weak_sig_map: ep.weak_sig_map.clone(),
            };
            (
                sig_uniq,
                AbortableTimedSharedFuture::new(
                    ep.config.timeout,
                    Error::str("Timeout awaiting signal server connection")
                        .into(),
                    Sig::new(_sig_drop, ep, sig_uniq, sig_url),
                ),
            )
        })
        .clone();

    match fut.await {
        Err(err) => {
            // if a new sig got added in the mean time, return that instead
            let r = sig_map.lock().unwrap().get(sig_url).cloned();

            if let Some((new_sig_uniq, new_sig_fut)) = r {
                if new_sig_uniq != sig_uniq {
                    return new_sig_fut.await;
                }
            }

            Err(err)
        }
        Ok(r) => Ok(r),
    }
}

fn close_sig(
    weak_sig_map: &Weak<Mutex<SigMap>>,
    sig_url: &SigUrl,
    close_sig_uniq: u64,
) {
    let mut tmp = None;

    if let Some(sig_map) = weak_sig_map.upgrade() {
        let mut lock = sig_map.lock().unwrap();
        if let Some((sig_uniq, sig)) = lock.remove(sig_url) {
            if close_sig_uniq != sig_uniq {
                // most of the time we'll be closing the real one,
                // so optimize for that case, and cause a hash probe
                // in the less likely case some race caused us to
                // try to remove the wrong one.
                tmp = lock.insert(sig_url.clone(), (sig_uniq, sig));
            } else {
                tmp = Some((sig_uniq, sig));
            }
        }
    }

    // make sure nothing is dropped while we're holding the mutex lock
    if let Some((_sig_uniq, sig_fut)) = tmp {
        sig_fut.abort(Error::id("Close").into());
        drop(sig_fut);
    }
}

pub(crate) mod sig;
pub(crate) use sig::*;

pub(crate) mod peer;
pub(crate) use peer::*;

#[cfg(test)]
mod test;
