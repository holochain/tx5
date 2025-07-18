use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// The backend webrtc module to use.
#[derive(Debug, Clone, Copy)]
pub enum BackendModule {
    /// Use the libdatachannel backend.
    #[cfg(feature = "backend-libdatachannel")]
    LibDataChannel,

    /// Use the go pion backend.
    #[cfg(feature = "backend-go-pion")]
    GoPion,
}

impl Default for BackendModule {
    #[allow(unreachable_code)]
    fn default() -> Self {
        #[cfg(feature = "backend-libdatachannel")]
        return Self::LibDataChannel;
        #[cfg(feature = "backend-go-pion")]
        Self::GoPion
    }
}

/// Tx5 connection hub config.
#[derive(Default)]
pub struct HubConfig {
    /// The backend webrtc module to use.
    pub backend_module: BackendModule,

    /// The signal config to use.
    pub signal_config: Arc<tx5_signal::SignalConfig>,

    /// Test falling back to the signal relay by failing the WebRTC setup.
    pub danger_force_signal_relay: bool,
}

/// The type of credential to use for ICE servers.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum CredentialType {
    /// A password is used for authentication.
    #[default]
    Password,
}

/// Configuration for a group of ICE servers.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct IceServers {
    /// The ICE server URLs to use for discovering external candidates.
    pub urls: Vec<String>,

    /// The username to use for authentication.
    #[serde(default)]
    pub username: Option<String>,

    /// The credential to use for authentication.
    #[serde(default)]
    pub credential: Option<String>,

    /// The credential type to use for authentication.
    #[serde(default)]
    pub credential_type: Option<CredentialType>,
}

/// ICE transport policy.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum TransportPolicy {
    /// Any type of candidate can be used.
    #[default]
    All,
    /// Only media relay candidates can be used.
    Relay,
}

/// WebRTC config.
///
/// This configuration will be mapped the specific configuration used by
/// the selected backend.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct WebRtcConfig {
    /// A list of ICE servers configurations.
    pub ice_servers: Vec<IceServers>,

    /// The ICE transport policy to use.
    #[serde(default)]
    pub ice_transport_policy: TransportPolicy,
}

#[cfg(feature = "backend-go-pion")]
impl WebRtcConfig {
    /// Convert this [`WebRtcConfig`] to a [`GoBuf`](tx5_go_pion::GoBuf).
    pub fn to_go_buf(&self) -> std::io::Result<tx5_go_pion::GoBuf> {
        serde_json::to_vec(self)
            .map_err(|e| {
                std::io::Error::other(format!(
                    "failed to serialize WebRtcConfig: {}",
                    e
                ))
            })?
            .try_into()
    }
}
