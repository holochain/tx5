use crate::*;
use once_cell::sync::OnceCell;
use std::sync::Arc;
use tx4_core::Tx4Url;

/// Tx4 config trait.
pub trait Config: 'static + Send + Sync {
    /// A request to open a new signal server connection.
    fn on_new_sig(&self, sig_url: Tx4Url, seed: state::SigStateSeed);

    /// A request to open a new peer connection.
    fn on_new_conn(&self, seed: state::ConnStateSeed);

    /// Request the lair tag associated with this config.
    fn lair_tag(&self) -> &Arc<str>;
}

/// Default tx4 config struct. Generally this is what you want.
#[derive(Default)]
pub struct DefConfig {
    lair_tag: OnceCell<Arc<str>>,
}

impl DefConfig {
    /// Set the lair tag. This can only be called once, and only before
    /// a call to `lair_tag()` or a default value will already have
    /// been applied.
    pub fn set_lair_tag(&mut self, lair_tag: Arc<str>) -> Result<()> {
        self.lair_tag
            .set(lair_tag)
            .map_err(|_| Error::id("AlreadySet"))
    }

    /// See `set_lair_tag()`, this is the builder version.
    pub fn with_lair_tag(mut self, lair_tag: Arc<str>) -> Result<Self> {
        self.set_lair_tag(lair_tag)?;
        Ok(self)
    }
}

impl Config for DefConfig {
    fn on_new_sig(&self, _sig_url: Tx4Url, _seed: state::SigStateSeed) {}

    fn on_new_conn(&self, _seed: state::ConnStateSeed) {}

    fn lair_tag(&self) -> &Arc<str> {
        self.lair_tag.get_or_init(|| {
            rand_utf8::rand_utf8(&mut rand::thread_rng(), 32).into()
        })
    }
}
