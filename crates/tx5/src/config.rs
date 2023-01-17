use crate::deps::lair_keystore_api;
use crate::deps::sodoken;
use crate::*;
use lair_keystore_api::prelude::*;
use std::sync::{Arc, Weak};
use tx5_core::{BoxFut, Tx5Url};

/// Tx5 config trait.
pub trait Config: 'static + Send + Sync {
    /// Get the max pending send byte count limit.
    fn max_send_bytes(&self) -> u32;

    /// Get the max queued recv byte count limit.
    fn max_recv_bytes(&self) -> u32;

    /// Get the max concurrent connection limit.
    fn max_conn_count(&self) -> u32;

    /// Get the max init (connect) time for a connection.
    fn max_conn_init(&self) -> std::time::Duration;

    /// Request the prometheus registry used by this config.
    fn metrics(&self) -> &prometheus::Registry;

    /// Request the lair client associated with this config.
    fn lair_client(&self) -> &LairClient;

    /// Request the lair tag associated with this config.
    fn lair_tag(&self) -> &Arc<str>;

    /// A request to open a new signal server connection.
    fn on_new_sig(&self, sig_url: Tx5Url, seed: state::SigStateSeed);

    /// A request to open a new peer connection.
    fn on_new_conn(
        &self,
        ice_servers: serde_json::Value,
        seed: state::ConnStateSeed,
    );
}

/// Dynamic config type alias.
pub type DynConfig = Arc<dyn Config + 'static + Send + Sync>;

impl std::fmt::Debug for dyn Config + 'static + Send + Sync {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("max_send_bytes", &self.max_send_bytes())
            .field("max_recv_bytes", &self.max_recv_bytes())
            .field("max_conn_count", &self.max_conn_count())
            .finish()
    }
}

/// Indicates a type is capable of being converted into a Config type.
pub trait IntoConfig: 'static + Send + Sync {
    /// Convert this type into a concrete config type.
    fn into_config(self) -> BoxFut<'static, Result<DynConfig>>;
}

struct DefConfigBuilt {
    this: Weak<Self>,
    max_send_bytes: u32,
    max_recv_bytes: u32,
    max_conn_count: u32,
    max_conn_init: std::time::Duration,
    metrics: prometheus::Registry,
    _lair_keystore: Option<lair_keystore_api::in_proc_keystore::InProcKeystore>,
    lair_client: LairClient,
    lair_tag: Arc<str>,
    on_new_sig_cb: Arc<
        dyn Fn(DynConfig, Tx5Url, state::SigStateSeed) + 'static + Send + Sync,
    >,
    on_new_conn_cb: Arc<
        dyn Fn(DynConfig, serde_json::Value, state::ConnStateSeed)
            + 'static
            + Send
            + Sync,
    >,
}

impl Config for DefConfigBuilt {
    fn max_send_bytes(&self) -> u32 {
        self.max_send_bytes
    }

    fn max_recv_bytes(&self) -> u32 {
        self.max_recv_bytes
    }

    fn max_conn_count(&self) -> u32 {
        self.max_conn_count
    }

    fn max_conn_init(&self) -> std::time::Duration {
        self.max_conn_init
    }

    fn metrics(&self) -> &prometheus::Registry {
        &self.metrics
    }

    fn lair_client(&self) -> &LairClient {
        &self.lair_client
    }

    fn lair_tag(&self) -> &Arc<str> {
        &self.lair_tag
    }

    fn on_new_sig(&self, sig_url: Tx5Url, seed: state::SigStateSeed) {
        if let Some(this) = self.this.upgrade() {
            (self.on_new_sig_cb)(this, sig_url, seed);
        }
    }

    fn on_new_conn(
        &self,
        ice_servers: serde_json::Value,
        seed: state::ConnStateSeed,
    ) {
        if let Some(this) = self.this.upgrade() {
            (self.on_new_conn_cb)(this, ice_servers, seed);
        }
    }
}

/// Builder type for constructing a DefConfig for a Tx5 endpoint.
#[derive(Default)]
#[allow(clippy::type_complexity)]
pub struct DefConfig {
    max_send_bytes: Option<u32>,
    max_recv_bytes: Option<u32>,
    max_conn_count: Option<u32>,
    max_conn_init: Option<std::time::Duration>,
    metrics: Option<prometheus::Registry>,
    lair_client: Option<LairClient>,
    lair_tag: Option<Arc<str>>,
    on_new_sig_cb: Option<
        Arc<
            dyn Fn(DynConfig, Tx5Url, state::SigStateSeed)
                + 'static
                + Send
                + Sync,
        >,
    >,
    on_new_conn_cb: Option<
        Arc<
            dyn Fn(DynConfig, serde_json::Value, state::ConnStateSeed)
                + 'static
                + Send
                + Sync,
        >,
    >,
}

impl IntoConfig for DefConfig {
    fn into_config(self) -> BoxFut<'static, Result<DynConfig>> {
        Box::pin(async move {
            let max_send_bytes =
                self.max_send_bytes.unwrap_or(16 * 1024 * 1024);
            let max_recv_bytes =
                self.max_recv_bytes.unwrap_or(16 * 1024 * 1024);
            let max_conn_count = self.max_conn_count.unwrap_or(40);
            let max_conn_init = self
                .max_conn_init
                .unwrap_or(std::time::Duration::from_secs(20));
            let metrics = self
                .metrics
                .unwrap_or_else(|| prometheus::default_registry().clone());
            let mut lair_keystore = None;

            let lair_tag = self.lair_tag.unwrap_or_else(|| {
                rand_utf8::rand_utf8(&mut rand::thread_rng(), 32).into()
            });

            let lair_client = match self.lair_client {
                Some(lair_client) => lair_client,
                None => {
                    let passphrase = sodoken::BufRead::new_no_lock(
                        rand_utf8::rand_utf8(&mut rand::thread_rng(), 32)
                            .as_bytes(),
                    );

                    // this is a memory keystore,
                    // so weak persistence security is okay,
                    // since it will not be persisted.
                    // The private keys will still be mem_locked
                    // so they shouldn't be swapped to disk.
                    let keystore_config = PwHashLimits::Minimum
                        .with_exec(|| {
                            LairServerConfigInner::new("/", passphrase.clone())
                        })
                        .await
                        .unwrap();

                    let keystore = PwHashLimits::Minimum
                        .with_exec(|| {
                            lair_keystore_api::in_proc_keystore::InProcKeystore::new(
                                Arc::new(keystore_config),
                                lair_keystore_api::mem_store::create_mem_store_factory(),
                                passphrase,
                            )
                        })
                        .await
                        .unwrap();

                    let lair_client = keystore.new_client().await.unwrap();

                    lair_client
                        .new_seed(lair_tag.clone(), None, false)
                        .await
                        .unwrap();

                    lair_keystore = Some(keystore);

                    lair_client
                }
            };

            let on_new_sig_cb = self
                .on_new_sig_cb
                .unwrap_or_else(|| Arc::new(endpoint::on_new_sig));

            let on_new_conn_cb = self
                .on_new_conn_cb
                .unwrap_or_else(|| Arc::new(endpoint::on_new_conn));

            let out: DynConfig = Arc::new_cyclic(|this| DefConfigBuilt {
                this: this.clone(),
                max_send_bytes,
                max_recv_bytes,
                max_conn_count,
                max_conn_init,
                metrics,
                _lair_keystore: lair_keystore,
                lair_client,
                lair_tag,
                on_new_sig_cb,
                on_new_conn_cb,
            });

            Ok(out)
        })
    }
}

impl DefConfig {
    /// Set the max queued send bytes to hold before applying backpressure.
    /// The default is `16 * 1024 * 1024`.
    pub fn set_max_send_bytes(&mut self, max_send_bytes: u32) {
        self.max_send_bytes = Some(max_send_bytes);
    }

    /// See `set_max_send_bytes()`, this is the builder version.
    pub fn with_max_send_bytes(mut self, max_send_bytes: u32) -> Self {
        self.set_max_send_bytes(max_send_bytes);
        self
    }

    /// Set the max queued recv bytes to hold before dropping connection.
    /// The default is `16 * 1024 * 1024`.
    pub fn set_max_recv_bytes(&mut self, max_recv_bytes: u32) {
        self.max_recv_bytes = Some(max_recv_bytes);
    }

    /// See `set_max_recv_bytes()`, this is the builder version.
    pub fn with_max_recv_bytes(mut self, max_recv_bytes: u32) -> Self {
        self.set_max_recv_bytes(max_recv_bytes);
        self
    }

    /// Set the max concurrent connection count.
    /// The default is `40`.
    pub fn set_max_conn_count(&mut self, max_conn_count: u32) {
        self.max_conn_count = Some(max_conn_count);
    }

    /// See `set_max_conn_count()`, this is the builder version.
    pub fn with_max_conn_count(mut self, max_conn_count: u32) -> Self {
        self.set_max_conn_count(max_conn_count);
        self
    }

    /// Set the max connection init (connect) time.
    /// The default is `20` seconds.
    pub fn set_max_conn_init(&mut self, max_conn_init: std::time::Duration) {
        self.max_conn_init = Some(max_conn_init);
    }

    /// See `set_max_conn_init()`, this is the builder version.
    pub fn with_max_conn_init(
        mut self,
        max_conn_init: std::time::Duration,
    ) -> Self {
        self.set_max_conn_init(max_conn_init);
        self
    }

    /// Set the prometheus metrics registry to use.
    /// The default is the global static default registry.
    pub fn set_metrics(&mut self, metrics: prometheus::Registry) {
        self.metrics = Some(metrics);
    }

    /// See `set_metrics()`, this is the builder version.
    pub fn with_metrics(mut self, metrics: prometheus::Registry) -> Self {
        self.set_metrics(metrics);
        self
    }

    /// Set the lair client.
    /// The default is a generated in-process, in-memory only keystore.
    pub fn set_lair_client(&mut self, lair_client: LairClient) {
        self.lair_client = Some(lair_client);
    }

    /// See `set_lair_client()`, this is the builder version.
    pub fn with_lair_client(mut self, lair_client: LairClient) -> Self {
        self.set_lair_client(lair_client);
        self
    }

    /// Set the lair tag used to identify the signing identity keypair.
    /// The default is a random 32 byte utf8 string.
    pub fn set_lair_tag(&mut self, lair_tag: Arc<str>) {
        self.lair_tag = Some(lair_tag);
    }

    /// See `set_lair_tag()`, this is the builder version.
    pub fn with_lair_tag(mut self, lair_tag: Arc<str>) -> Self {
        self.set_lair_tag(lair_tag);
        self
    }

    /// Override the default new signal connection request handler.
    /// The default uses the default tx5-signal dependency.
    pub fn set_new_sig_cb<Cb>(&mut self, cb: Cb)
    where
        Cb: Fn(DynConfig, Tx5Url, state::SigStateSeed) + 'static + Send + Sync,
    {
        self.on_new_sig_cb = Some(Arc::new(cb));
    }

    /// See `set_new_sig_cb()`, this is the builder version.
    pub fn with_new_sig_cb<Cb>(mut self, cb: Cb) -> Self
    where
        Cb: Fn(DynConfig, Tx5Url, state::SigStateSeed) + 'static + Send + Sync,
    {
        self.set_new_sig_cb(cb);
        self
    }

    /// Override the default new peer connection request handler.
    /// The default uses either tx5-go-pion, or rust-webrtc depending
    /// on the feature flipper chosen at compile time.
    pub fn set_new_conn_cb<Cb>(&mut self, cb: Cb)
    where
        Cb: Fn(DynConfig, serde_json::Value, state::ConnStateSeed)
            + 'static
            + Send
            + Sync,
    {
        self.on_new_conn_cb = Some(Arc::new(cb));
    }

    /// See `set_new_conn_cb()`, this is the builder version.
    pub fn with_new_conn_cb<Cb>(mut self, cb: Cb) -> Self
    where
        Cb: Fn(DynConfig, serde_json::Value, state::ConnStateSeed)
            + 'static
            + Send
            + Sync,
    {
        self.set_new_conn_cb(cb);
        self
    }
}
