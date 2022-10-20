use crate::deps::lair_keystore_api;
use crate::deps::sodoken;
use crate::*;
use lair_keystore_api::prelude::*;
use std::sync::{Arc, Weak};
use tx4_core::{BoxFut, Tx4Url};

/// Tx4 config trait.
pub trait Config: 'static + Send + Sync {
    /// Request the lair client associated with this config.
    fn lair_client(&self) -> &LairClient;

    /// Request the lair tag associated with this config.
    fn lair_tag(&self) -> &Arc<str>;

    /// A request to open a new signal server connection.
    fn on_new_sig(&self, sig_url: Tx4Url, seed: state::SigStateSeed);

    /// A request to open a new peer connection.
    fn on_new_conn(
        &self,
        ice_servers: serde_json::Value,
        seed: state::ConnStateSeed,
    );
}

/// Indicates a type is capable of being converted into a Config type.
pub trait IntoConfig: 'static + Send + Sync {
    /// The concrete config type this type can be converted into.
    type Config: Config;

    /// Convert this type into a concrete config type.
    fn into_config(self) -> BoxFut<'static, Result<Arc<Self::Config>>>;
}

/// The default config type.
pub struct DefConfigBuilt {
    this: Weak<Self>,
    _lair_keystore: Option<lair_keystore_api::in_proc_keystore::InProcKeystore>,
    lair_client: LairClient,
    lair_tag: Arc<str>,
    on_new_sig_cb: Arc<
        dyn Fn(Arc<Self>, Tx4Url, state::SigStateSeed) + 'static + Send + Sync,
    >,
    on_new_conn_cb: Arc<
        dyn Fn(Arc<Self>, serde_json::Value, state::ConnStateSeed)
            + 'static
            + Send
            + Sync,
    >,
}

impl IntoConfig for DefConfigBuilt {
    type Config = DefConfigBuilt;

    fn into_config(mut self) -> BoxFut<'static, Result<Arc<Self::Config>>> {
        Box::pin(async move {
            Ok(Arc::new_cyclic(|this| {
                // we were unwrapped... so this "this" is no longer valid
                // we need to use the newly created one.
                self.this = this.clone();
                self
            }))
        })
    }
}

impl Config for DefConfigBuilt {
    fn lair_client(&self) -> &LairClient {
        &self.lair_client
    }

    fn lair_tag(&self) -> &Arc<str> {
        &self.lair_tag
    }

    fn on_new_sig(&self, sig_url: Tx4Url, seed: state::SigStateSeed) {
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

/// Builder type for constructing a DefConfig for a Tx4 endpoint.
#[derive(Default)]
pub struct DefConfig {
    lair_client: Option<LairClient>,
    lair_tag: Option<Arc<str>>,
    on_new_sig_cb: Option<
        Arc<
            dyn Fn(Arc<DefConfigBuilt>, Tx4Url, state::SigStateSeed)
                + 'static
                + Send
                + Sync,
        >,
    >,
    on_new_conn_cb: Option<
        Arc<
            dyn Fn(Arc<DefConfigBuilt>, serde_json::Value, state::ConnStateSeed)
                + 'static
                + Send
                + Sync,
        >,
    >,
}

impl IntoConfig for DefConfig {
    type Config = DefConfigBuilt;

    fn into_config(self) -> BoxFut<'static, Result<Arc<Self::Config>>> {
        Box::pin(async move {
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

            Ok(Arc::new_cyclic(|this| DefConfigBuilt {
                this: this.clone(),
                _lair_keystore: lair_keystore,
                lair_client,
                lair_tag,
                on_new_sig_cb,
                on_new_conn_cb,
            }))
        })
    }
}

impl DefConfig {
    /// Set the lair client.
    pub fn set_lair_client(&mut self, lair_client: LairClient) {
        self.lair_client = Some(lair_client);
    }

    /// See `set_lair_client()`, this is the builder version.
    pub fn with_lair_client(mut self, lair_client: LairClient) -> Self {
        self.set_lair_client(lair_client);
        self
    }

    /// Set the lair tag used to identify the signing identity keypair.
    pub fn set_lair_tag(&mut self, lair_tag: Arc<str>) {
        self.lair_tag = Some(lair_tag);
    }

    /// See `set_lair_tag()`, this is the builder version.
    pub fn with_lair_tag(mut self, lair_tag: Arc<str>) -> Self {
        self.set_lair_tag(lair_tag);
        self
    }

    /// Override the default new signal connection request handler.
    pub fn set_new_sig_cb<Cb>(&mut self, cb: Cb)
    where
        Cb: Fn(Arc<DefConfigBuilt>, Tx4Url, state::SigStateSeed)
            + 'static
            + Send
            + Sync,
    {
        self.on_new_sig_cb = Some(Arc::new(cb));
    }

    /// See `set_new_sig_cb()`, this is the builder version.
    pub fn with_new_sig_cb<Cb>(mut self, cb: Cb) -> Self
    where
        Cb: Fn(Arc<DefConfigBuilt>, Tx4Url, state::SigStateSeed)
            + 'static
            + Send
            + Sync,
    {
        self.set_new_sig_cb(cb);
        self
    }

    /// Override the default new peer connection request handler.
    pub fn set_new_conn_cb<Cb>(&mut self, cb: Cb)
    where
        Cb: Fn(Arc<DefConfigBuilt>, serde_json::Value, state::ConnStateSeed)
            + 'static
            + Send
            + Sync,
    {
        self.on_new_conn_cb = Some(Arc::new(cb));
    }

    /// See `set_new_conn_cb()`, this is the builder version.
    pub fn with_new_conn_cb<Cb>(mut self, cb: Cb) -> Self
    where
        Cb: Fn(Arc<DefConfigBuilt>, serde_json::Value, state::ConnStateSeed)
            + 'static
            + Send
            + Sync,
    {
        self.set_new_conn_cb(cb);
        self
    }
}
