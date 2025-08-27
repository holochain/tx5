use crate::*;
use tx5_connection::WebRtcConfig;
use tx5_core::deps::serde_json;

/// Tx5 endpoint configuration.
#[derive(Clone)]
#[non_exhaustive]
pub struct Config {
    /// Allow plain text (non-tls) signal server connections.
    ///
    /// Default: false (require TLS).
    pub signal_allow_plain_text: bool,

    /// Supply signal authentication material.
    ///
    /// Default: None (no authentication).
    pub signal_auth_material: Option<Vec<u8>>,

    /// Initial webrtc peer connection config.
    pub initial_webrtc_config: WebRtcConfig,

    /// Maximum count of open connections.
    ///
    /// Default 4096.
    pub connection_count_max: u32,

    /// Max backend send buffer bytes (per connection).
    ///
    /// Default: 64 KiB.
    pub send_buffer_bytes_max: u32,

    /// Max backend recv buffer bytes (per connection).
    ///
    /// Default: 64 KiB.
    pub recv_buffer_bytes_max: u32,

    /// Maximum receive message reconstruction bytes in memory
    /// (across entire endpoint).
    ///
    /// Default: 512 MiB.
    pub incoming_message_bytes_max: u32,

    /// Maximum size of an individual message.
    ///
    /// Default: 16 MiB.
    pub message_size_max: u32,

    /// Internal event channel size.
    ///
    /// Default: 1024.
    pub internal_event_channel_size: u32,

    /// Default timeout for network operations.
    ///
    /// Default: 60 seconds.
    pub timeout: std::time::Duration,

    /// The timeout for establishing a WebRTC connection to a peer.
    ///
    /// This value *must* be less than [`Config::timeout`], otherwise the connection attempt will
    /// be treated as failed without attempting to fall back to the signal relay.
    ///
    /// A lower value for this timeout will make tx5 fall back to the signal relay faster. That
    /// makes tx5 more responsive when direct connections are not possible. It also increases
    /// the chance that connections end up being relayed unnecessarily when a direct connection
    /// could have been established with a bit more time.
    ///
    /// The default of 45 seconds is a trade-off that gives WebRTC plenty of time to establish a
    /// direct connection in most situations while still leaving some time for the signal relay to
    /// be used within the overall timeout.
    ///
    /// Default: 45 seconds.
    pub webrtc_connect_timeout: std::time::Duration,

    /// Starting backoff duration for retries.
    ///
    /// Default: 5 seconds.
    pub backoff_start: std::time::Duration,

    /// Max backoff duration for retries.
    ///
    /// Default: 60 seconds.
    pub backoff_max: std::time::Duration,

    /// If the protocol should manage a preflight message,
    /// set the callbacks here, otherwise no preflight will
    /// be sent nor validated.
    ///
    /// Default: None.
    pub preflight: Option<(PreflightSendCb, PreflightCheckCb)>,

    /// The backend connection module to use.
    ///
    /// For the most part you should just leave this at the default.
    pub backend_module: crate::backend::BackendModule,

    /// The backend module config to use.
    ///
    /// For the most part you should just leave this set at `None`,
    /// to get the default backend config.
    pub backend_module_config: Option<serde_json::Value>,

    /// Test falling back to the signal relay by failing the WebRTC setup.
    pub danger_force_signal_relay: bool,

    /// Deny using the signal server as a relay if direct connections fail.
    ///
    /// This is useful if you want to ensure that the signal server is only used for connection
    /// establishment and not as a relay.
    ///
    /// Enabling the setting is dangerous though, as it will mean that if a peer fails to make a
    /// direct connection, it will not be able to communicate at all. For some peers, this might
    /// mean they can make no connections at all, and are entirely isolated from the network.
    pub danger_deny_signal_relay: bool,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let signal_auth_material = if self.signal_auth_material.is_some() {
            "some"
        } else {
            "none"
        };
        f.debug_struct("Config")
            .field("signal_allow_plain_text", &self.signal_allow_plain_text)
            .field("signal_auth_material", &signal_auth_material)
            .field("initial_webrtc_config", &self.initial_webrtc_config)
            .field("connection_count_max", &self.connection_count_max)
            .field("send_buffer_bytes_max", &self.send_buffer_bytes_max)
            .field("recv_buffer_bytes_max", &self.recv_buffer_bytes_max)
            .field(
                "incoming_message_bytes_max",
                &self.incoming_message_bytes_max,
            )
            .field("message_size_max", &self.message_size_max)
            .field(
                "internal_event_channel_size",
                &self.internal_event_channel_size,
            )
            .field("timeout", &self.timeout)
            .field("backoff_start", &self.backoff_start)
            .field("backoff_max", &self.backoff_max)
            .finish()
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::new()
    }
}

impl Config {
    /// Create a new default config.
    pub fn new() -> Config {
        Self {
            signal_allow_plain_text: false,
            signal_auth_material: None,
            initial_webrtc_config: WebRtcConfig::default(),
            connection_count_max: 4096,
            send_buffer_bytes_max: 64 * 1024,
            recv_buffer_bytes_max: 64 * 1024,
            incoming_message_bytes_max: 512 * 1024 * 1024,
            message_size_max: 16 * 1024 * 1024,
            internal_event_channel_size: 1024,
            timeout: std::time::Duration::from_secs(60),
            webrtc_connect_timeout: std::time::Duration::from_secs(45),
            backoff_start: std::time::Duration::from_secs(5),
            backoff_max: std::time::Duration::from_secs(60),
            preflight: None,
            backend_module: BackendModule::default(),
            backend_module_config: None,
            danger_force_signal_relay: false,
            danger_deny_signal_relay: false,
        }
    }

    /// Set whether to allow plain text (non-tls) signal server connections.
    pub fn with_signal_allow_plain_text(mut self, allow: bool) -> Self {
        self.signal_allow_plain_text = allow;
        self
    }

    /// Set the signal authentication material.
    pub fn with_signal_auth_material(mut self, auth: Vec<u8>) -> Self {
        self.signal_auth_material = Some(auth);
        self
    }

    /// Set the initial webrtc peer connection config.
    pub fn with_initial_webrtc_config(mut self, config: WebRtcConfig) -> Self {
        self.initial_webrtc_config = config;
        self
    }

    /// Set the default timeout for network operations.
    pub fn with_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Set the timeout for establishing a WebRTC connection to a peer.
    pub fn with_webrtc_connect_timeout(
        mut self,
        timeout: std::time::Duration,
    ) -> Self {
        self.webrtc_connect_timeout = timeout;
        self
    }

    /// Set the preflight callbacks.
    pub fn with_preflight(
        mut self,
        send_cb: PreflightSendCb,
        check_cb: PreflightCheckCb,
    ) -> Self {
        self.preflight = Some((send_cb, check_cb));
        self
    }

    /// Set the backend connection module to use.
    pub fn with_backend_module(mut self, module: BackendModule) -> Self {
        self.backend_module = module;
        self
    }

    /// Set the backend module config to use.
    pub fn with_backend_module_config(
        mut self,
        config: serde_json::Value,
    ) -> Self {
        self.backend_module_config = Some(config);
        self
    }
}
