//! Module containing tx5 endpoint version 3 types.

use crate::deps::lair_keystore_api;
use crate::deps::sodoken;
use crate::BackBuf;
use futures::future::{BoxFuture, Shared};
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
    Error(Error),
}

impl From<Error> for Ep3Event {
    fn from(err: Error) -> Self {
        Self::Error(err)
    }
}

/// A signal server url.
pub type SigUrl = Tx5Url;

/// A peer connection url.
pub type PeerUrl = Tx5Url;

type SharedSig = Shared<BoxFuture<'static, CRes<Arc<Sig>>>>;
type SigMap = HashMap<SigUrl, (u64, SharedSig)>;

/// Tx5 endpoint version 3 configuration.
pub struct Config3 {
    /// Maximum count of open connections. Default 255.
    pub connection_count_max: u32,

    /// Maximum bytes in memory for any given connection. Default 16 MiB.
    pub connection_bytes_max: u32,

    /// Default timeout for network operations. Default 60 seconds.
    pub timeout: std::time::Duration,
}

impl Default for Config3 {
    fn default() -> Self {
        Self {
            connection_count_max: 255,
            connection_bytes_max: 16 * 1024 * 1024,
            timeout: std::time::Duration::from_secs(60),
        }
    }
}

/// Tx5 endpoint version 3.
pub struct Ep3 {
    ep_uniq: u64,
    config: Arc<Config3>,
    this_id: Id,
    sig_limit: Arc<tokio::sync::Semaphore>,
    peer_limit: Arc<tokio::sync::Semaphore>,
    sig_map: Arc<Mutex<SigMap>>,
    lair_tag: Arc<str>,
    lair_client: LairClient,
    _lair_keystore: lair_keystore_api::in_proc_keystore::InProcKeystore,
    listen_sigs: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
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
        let sig_limit = Arc::new(tokio::sync::Semaphore::new(
            config.connection_count_max as usize,
        ));

        let peer_limit = Arc::new(tokio::sync::Semaphore::new(
            config.connection_count_max as usize,
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

        let this = Self {
            ep_uniq: next_uniq(),
            config,
            this_id,
            sig_limit,
            peer_limit,
            sig_map: Arc::new(Mutex::new(HashMap::new())),
            lair_tag,
            lair_client,
            _lair_keystore,
            listen_sigs: Arc::new(Mutex::new(Vec::new())),
        };

        let (_evt_send, evt_recv) = EventSend::new(1024);

        (this, evt_recv)
    }

    /// Establish a listening connection to a signal server,
    /// from which we can accept incoming remote connections.
    /// Returns the client url at which this endpoint may now be addressed.
    pub fn listen(&self, sig_url: SigUrl) -> Result<PeerUrl> {
        if !sig_url.is_server() {
            return Err(Error::str("Expected SigUrl, got PeerUrl"));
        }

        let ep_uniq = self.ep_uniq;
        let peer_url = sig_url.to_client(self.this_id);

        let config = self.config.clone();
        let sig_limit = self.sig_limit.clone();
        let peer_limit = self.peer_limit.clone();
        let weak_sig_map = Arc::downgrade(&self.sig_map);

        let this_id = self.this_id;
        let lair_tag = self.lair_tag.clone();
        let lair_client = self.lair_client.clone();

        self.listen_sigs
            .lock()
            .unwrap()
            .push(tokio::task::spawn(async move {
                const B_START: std::time::Duration =
                    std::time::Duration::from_secs(5);
                const B_MAX: std::time::Duration =
                    std::time::Duration::from_secs(60);
                let mut backoff = B_START;
                loop {
                    if let Some(sig_map) = weak_sig_map.upgrade() {
                        if assert_sig(
                            ep_uniq,
                            &config,
                            &sig_limit,
                            &peer_limit,
                            &sig_map,
                            this_id,
                            &lair_tag,
                            &lair_client,
                            &sig_url,
                        )
                        .await
                        .is_ok()
                        {
                            // if the conn is still open it's essentially
                            // a no-op to assert it again, so it's
                            // okay to do that quickly.
                            backoff = B_START;
                        } else {
                            backoff *= 2;
                            if backoff > B_MAX {
                                backoff = B_MAX;
                            }
                        }
                    } else {
                        // endpoint closed
                        break;
                    }

                    tokio::time::sleep(backoff).await;
                }
            }));

        Ok(peer_url)
    }

    /// Send data to a remote on this tx5 endpoint.
    /// The future returned from this method will resolve when
    /// the data is handed off to our networking backend.
    pub async fn send(
        &self,
        peer_url: PeerUrl,
        _data: Vec<BackBuf>,
    ) -> Result<()> {
        if !peer_url.is_client() {
            return Err(Error::str("Expected PeerUrl, got SigUrl"));
        }

        let sig_url = peer_url.to_server();
        let peer_id = peer_url.id().unwrap();

        let sig = assert_sig(
            self.ep_uniq,
            &self.config,
            &self.sig_limit,
            &self.peer_limit,
            &self.sig_map,
            self.this_id,
            &self.lair_tag,
            &self.lair_client,
            &sig_url,
        )
        .await?;

        let _peer = sig.assert_peer(peer_id, PeerDir::Outgoing).await?;

        tokio::time::sleep(std::time::Duration::from_secs(10)).await;

        todo!()
    }
}

async fn assert_sig(
    ep_uniq: u64,
    config: &Arc<Config3>,
    sig_limit: &Arc<tokio::sync::Semaphore>,
    peer_limit: &Arc<tokio::sync::Semaphore>,
    sig_map: &Arc<Mutex<SigMap>>,
    this_id: Id,
    lair_tag: &Arc<str>,
    lair_client: &LairClient,
    sig_url: &SigUrl,
) -> CRes<Arc<Sig>> {
    let (_sig_uniq, fut) = sig_map
        .lock()
        .unwrap()
        .entry(sig_url.clone())
        .or_insert_with(|| {
            let sig_uniq = next_uniq();
            let config = config.clone();
            let sig_limit = sig_limit.clone();
            let peer_limit = peer_limit.clone();
            let weak_sig_map = Arc::downgrade(&sig_map);
            let sig_url = sig_url.clone();
            let lair_tag = lair_tag.clone();
            let lair_client = lair_client.clone();
            (
                sig_uniq,
                futures::future::FutureExt::shared(
                    futures::future::FutureExt::boxed(async move {
                        tokio::time::timeout(
                            config.timeout,
                            Sig::new(
                                ep_uniq,
                                sig_uniq,
                                config,
                                sig_limit,
                                peer_limit,
                                weak_sig_map,
                                this_id,
                                sig_url,
                                lair_tag,
                                lair_client,
                            ),
                        )
                        .await
                        .map_err(|_| {
                            Error::str(
                                "Timeout awaiting signal server connection",
                            )
                        })?
                    }),
                ),
            )
        })
        .clone();

    fut.await
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
    drop(tmp);
}

type SharedPeer = Shared<BoxFuture<'static, CRes<Arc<Peer>>>>;
type PeerMap = HashMap<Id, (u64, SharedPeer)>;

struct Sig {
    weak_sig: Weak<Sig>,
    ep_uniq: u64,
    sig_uniq: u64,
    config: Arc<Config3>,
    _permit: tokio::sync::OwnedSemaphorePermit,
    weak_sig_map: Weak<Mutex<SigMap>>,
    sig_url: SigUrl,
    recv_task: tokio::task::JoinHandle<()>,
    ice_servers: Arc<serde_json::Value>,
    peer_limit: Arc<tokio::sync::Semaphore>,
    peer_map: Arc<Mutex<PeerMap>>,
    sig: tx5_signal::Cli,
}

impl Drop for Sig {
    fn drop(&mut self) {
        tracing::info!(%self.ep_uniq, %self.sig_uniq, %self.sig_url, "Signal Connection Close");

        self.recv_task.abort();

        close_sig(&self.weak_sig_map, &self.sig_url, self.sig_uniq);
    }
}

impl std::ops::Deref for Sig {
    type Target = tx5_signal::Cli;

    fn deref(&self) -> &Self::Target {
        &self.sig
    }
}

impl Sig {
    pub async fn new(
        ep_uniq: u64,
        sig_uniq: u64,
        config: Arc<Config3>,
        sig_limit: Arc<tokio::sync::Semaphore>,
        peer_limit: Arc<tokio::sync::Semaphore>,
        weak_sig_map: Weak<Mutex<SigMap>>,
        this_id: Id,
        sig_url: SigUrl,
        lair_tag: Arc<str>,
        lair_client: LairClient,
    ) -> CRes<Arc<Self>> {
        tracing::info!(%ep_uniq, %sig_uniq, %sig_url, "Signal Connection Connecting");

        let _permit = sig_limit.acquire_owned().await.map_err(|_| {
            Error::str(
                "Endpoint closed while acquiring signal connection permit",
            )
        })?;

        let (sig, mut sig_recv) = tx5_signal::Cli::builder()
            .with_lair_tag(lair_tag)
            .with_lair_client(lair_client)
            .with_url(sig_url.to_string().parse().unwrap())
            .build()
            .await?;

        let peer_url = Tx5Url::new(sig.local_addr())?;
        if peer_url.id().unwrap() != this_id {
            return Err(Error::str("Invalid signal server peer Id").into());
        }

        let ice_servers = sig.ice_servers();

        Ok(Arc::new_cyclic(move |weak_sig: &Weak<Sig>| {
            let recv_task = {
                let weak_sig = weak_sig.clone();
                let weak_sig_map = weak_sig_map.clone();
                let sig_url = sig_url.clone();
                tokio::task::spawn(async move {
                    while let Some(msg) = sig_recv.recv().await {
                        use tx5_signal::SignalMsg::*;
                        match msg {
                            Demo { .. } => (),
                            Offer { rem_pub, offer } => {
                                tracing::trace!(%ep_uniq, %sig_uniq, ?rem_pub, ?offer, "Sig Recv Offer");
                                if let Some(sig) = weak_sig.upgrade() {
                                    // fire and forget this
                                    tokio::task::spawn(async move {
                                        let _ = sig.assert_peer(rem_pub, PeerDir::Incoming { offer }).await;
                                    });
                                } else {
                                    break;
                                }
                            }
                            Answer { rem_pub, answer } => {
                                tracing::trace!(%ep_uniq, %sig_uniq, ?rem_pub, ?answer, "Sig Recv Answer");
                            }
                            Ice { rem_pub, ice } => {
                                tracing::trace!(%ep_uniq, %sig_uniq, ?rem_pub, ?ice, "Sig Recv ICE");
                            }
                        }
                    }

                    close_sig(&weak_sig_map, &sig_url, sig_uniq);
                })
            };

            tracing::info!(%ep_uniq, %sig_uniq, %sig_url, "Signal Connection Open");

            Self {
                weak_sig: weak_sig.clone(),
                ep_uniq,
                sig_uniq,
                config,
                _permit,
                weak_sig_map,
                sig_url,
                recv_task,
                ice_servers,
                peer_limit,
                peer_map: Arc::new(Mutex::new(HashMap::new())),
                sig,
            }
        }))
    }

    async fn assert_peer(&self, peer_id: Id, peer_dir: PeerDir) -> CRes<Arc<Peer>> {
        let (_peer_uniq, fut) = self
            .peer_map
            .lock()
            .unwrap()
            .entry(peer_id)
            .or_insert_with(|| {
                let weak_sig = self.weak_sig.clone();
                let timeout = self.config.timeout;
                let ep_uniq = self.ep_uniq;
                let sig_uniq = self.sig_uniq;
                let peer_uniq = next_uniq();
                let config = self.config.clone();
                let peer_limit = self.peer_limit.clone();
                let weak_peer_map = Arc::downgrade(&self.peer_map);
                let ice_servers = self.ice_servers.clone();
                (
                    peer_uniq,
                    futures::future::FutureExt::shared(
                        futures::future::FutureExt::boxed(async move {
                            tokio::time::timeout(
                                timeout,
                                Peer::new(
                                    weak_sig,
                                    peer_id,
                                    ep_uniq,
                                    sig_uniq,
                                    peer_uniq,
                                    config,
                                    peer_limit,
                                    weak_peer_map,
                                    ice_servers,
                                    peer_dir,
                                ),
                            )
                            .await
                            .map_err(|_| {
                                Error::str("Timeout awaiting peer connection")
                            })?
                        }),
                    ),
                )
            })
            .clone();

        fut.await
    }
}

fn close_peer(
    weak_peer_map: &Weak<Mutex<PeerMap>>,
    peer_id: Id,
    close_peer_uniq: u64,
) {
    let mut tmp = None;

    if let Some(peer_map) = weak_peer_map.upgrade() {
        let mut lock = peer_map.lock().unwrap();
        if let Some((peer_uniq, peer)) = lock.remove(&peer_id) {
            if close_peer_uniq != peer_uniq {
                // most of the time we'll be closing the real one,
                // so optimize for that case, and cause a hash probe
                // in the less likely case some race caused us to
                // try to remove the wrong one.
                tmp = lock.insert(peer_id, (peer_uniq, peer));
            } else {
                tmp = Some((peer_uniq, peer));
            }
        }
    }

    // make sure nothing is dropped while we're holding the mutex lock
    drop(tmp);
}

enum PeerDir {
    Outgoing,
    Incoming {
        offer: serde_json::Value,
    },
}

struct Peer {
    peer_id: Id,
    ep_uniq: u64,
    sig_uniq: u64,
    peer_uniq: u64,
    _permit: tokio::sync::OwnedSemaphorePermit,
    weak_peer_map: Weak<Mutex<PeerMap>>,
    recv_task: tokio::task::JoinHandle<()>,
}

impl Drop for Peer {
    fn drop(&mut self) {
        tracing::info!(%self.ep_uniq, %self.sig_uniq, %self.peer_uniq, ?self.peer_id, "Peer Connection Close");

        self.recv_task.abort();

        close_peer(&self.weak_peer_map, self.peer_id, self.peer_uniq);
    }
}

impl Peer {
    pub async fn new(
        weak_sig: Weak<Sig>,
        peer_id: Id,
        ep_uniq: u64,
        sig_uniq: u64,
        peer_uniq: u64,
        config: Arc<Config3>,
        peer_limit: Arc<tokio::sync::Semaphore>,
        weak_peer_map: Weak<Mutex<PeerMap>>,
        ice_servers: Arc<serde_json::Value>,
        peer_dir: PeerDir,
    ) -> CRes<Arc<Self>> {
        tracing::info!(%ep_uniq, %sig_uniq, %peer_uniq, ?peer_id, "Peer Connection Connecting");

        let _permit = peer_limit.acquire_owned().await.map_err(|_| {
            Error::str(
                "Endpoint closed while acquiring peer connection permit",
            )
        })?;

        let sig = match weak_sig.upgrade() {
            None => return Err(Error::str("Sig shutdown while opening peer connection").into()),
            Some(sig) => sig,
        };

        let peer_config = BackBuf::from_json(ice_servers)?;

        let (peer, mut peer_recv) = tx5_go_pion::PeerConnection::new(
            peer_config.imp.buf,
            Arc::new(tokio::sync::Semaphore::new(
                config.connection_bytes_max as usize,
            )),
        )
        .await?;

        match peer_dir {
            PeerDir::Outgoing => {
                let (_chan, _chan_recv) = peer
                    .create_data_channel(tx5_go_pion::DataChannelConfig {
                        label: Some("data".into()),
                    })
                    .await?;

                let mut offer = peer
                    .create_offer(tx5_go_pion::OfferConfig::default())
                    .await?;

                let offer_json = offer.as_json()?;

                tracing::debug!(?offer_json, "create_offer");

                sig.offer(peer_id, offer_json).await?;

                peer.set_local_description(offer).await?;
            }
            PeerDir::Incoming { offer } => {
                let offer = BackBuf::from_json(offer)?;

                peer.set_remote_description(offer.imp.buf).await?;

                let mut answer = peer
                    .create_answer(tx5_go_pion::AnswerConfig::default())
                    .await?;

                let answer_json = answer.as_json()?;

                tracing::debug!(?answer_json, "create_answer");

                sig.answer(peer_id, answer_json).await?;

                peer.set_local_description(answer).await?;
            }
        }

        let recv_task = {
            let weak_peer_map = weak_peer_map.clone();
            tokio::task::spawn(async move {
                while let Some(_evt) = peer_recv.recv().await {}

                close_peer(&weak_peer_map, peer_id, peer_uniq);
            })
        };

        tracing::info!(%ep_uniq, %sig_uniq, %peer_uniq, ?peer_id, "Peer Connection Open");

        Ok(Arc::new(Self {
            peer_id,
            ep_uniq,
            sig_uniq,
            peer_uniq,
            _permit,
            weak_peer_map,
            recv_task,
        }))
    }
}

#[cfg(test)]
mod ep3_tests {
    use super::*;

    fn init_tracing() {
        let subscriber = tracing_subscriber::FmtSubscriber::builder()
            .with_env_filter(
                tracing_subscriber::filter::EnvFilter::from_default_env(),
            )
            .with_file(true)
            .with_line_number(true)
            .finish();
        let _ = tracing::subscriber::set_global_default(subscriber);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ep3_sanity() {
        init_tracing();

        let mut srv_config = tx5_signal_srv::Config::default();
        srv_config.port = 0;

        let (srv_driver, addr_list, _) =
            tx5_signal_srv::exec_tx5_signal_srv(srv_config).unwrap();
        let sig_task = tokio::task::spawn(srv_driver);

        let sig_port = addr_list.get(0).unwrap().port();

        let sig_url =
            Tx5Url::new(format!("ws://localhost:{}", sig_port)).unwrap();
        println!("sig_url: {sig_url}");

        let (ep1, _) = Ep3::new(Arc::new(Config3::default())).await;

        let cli_url1 = ep1.listen(sig_url.clone()).unwrap();
        println!("cli_url1: {cli_url1}");

        let (ep2, _) = Ep3::new(Arc::new(Config3::default())).await;

        let cli_url2 = ep2.listen(sig_url).unwrap();
        println!("cli_url2: {cli_url2}");

        ep1.send(cli_url2, vec![BackBuf::from_slice(b"hello").unwrap()])
            .await
            .unwrap();

        println!("drop1");
        drop(ep1);
        println!("drop2");
        drop(ep2);

        println!("abort sig");
        sig_task.abort();

        println!("all done.");
    }
}
