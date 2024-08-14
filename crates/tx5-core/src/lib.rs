#![deny(missing_docs)]
#![deny(unsafe_code)]
#![doc = include_str!("README.tpl")]
//! # tx5-core
//!
//! Holochain WebRTC p2p communication ecosystem core types.

include!(concat!(env!("OUT_DIR"), "/readme.rs"));

/// Re-exported dependencies.
pub mod deps {
    pub use base64;
    pub use serde;
    pub use serde_json;
}

mod error;
pub use error::*;

mod evt;
pub use evt::*;

#[cfg(feature = "file_check")]
pub mod file_check;

/// Pinned, boxed, future type alias.
pub type BoxFut<'lt, T> =
    std::pin::Pin<Box<dyn std::future::Future<Output = T> + 'lt + Send>>;

/// Initial configuration. If you would like to change this from the
/// default, please call [Tx5InitConfig::set_as_global_default]
/// before creating any peer connections.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
#[serde(crate = "deps::serde", rename_all = "camelCase")]
pub struct Tx5InitConfig {
    /// If true, tracing logs from the backend webrtc library will be included.
    /// Defaults to `false`.
    pub tracing_enabled: bool,

    /// The minimum ephemeral udp port to bind. Defaults to `1`.
    pub ephemeral_udp_port_min: u16,

    /// The maximum ephemeral udp port to bind. Defaults to `65535`.
    pub ephemeral_udp_port_max: u16,

    /// The maximum time allowed for application calls to complete.
    /// Defaults to 999ms (~1 second).
    pub slow_app_timeout: std::time::Duration,
}

impl Default for Tx5InitConfig {
    fn default() -> Self {
        Self {
            tracing_enabled: false,
            ephemeral_udp_port_min: 1,
            ephemeral_udp_port_max: 65535,
            slow_app_timeout: std::time::Duration::from_millis(999),
        }
    }
}

impl Tx5InitConfig {
    /// Call this to set tx5_init defaults before creating any peer connections.
    /// This will return an error if the settings have already been set.
    pub fn set_as_global_default(&self) -> Result<()> {
        TX5_INIT_CONFIG
            .set(*self)
            .map_err(|_| Error::id("Tx5InitAlreadySet"))
    }

    /// Get the currently set Tx5InitConfig. WARNING! If it hasn't been
    /// explicitly set, this get will trigger the config to be set
    /// to default values.
    pub fn get() -> Self {
        *TX5_INIT_CONFIG.get_or_init(Tx5InitConfig::default)
    }
}

static TX5_INIT_CONFIG: once_cell::sync::OnceCell<Tx5InitConfig> =
    once_cell::sync::OnceCell::new();
