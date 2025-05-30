use crate::*;
use tx5_connection::WebRtcConfig;
use tx5_core::deps::serde_json;

/// Tx5 endpoint configuration.
pub struct Config {
    /// Allow plain text (non-tls) signal server connections.
    pub signal_allow_plain_text: bool,

    /// Supply signal authentication material.
    pub signal_auth_material: Option<Vec<u8>>,

    /// Initial webrtc peer connection config.
    pub initial_webrtc_config: WebRtcConfig,

    /// Maximum count of open connections. Default 4096.
    pub connection_count_max: u32,

    /// Max backend send buffer bytes (per connection). Default 64 KiB.
    pub send_buffer_bytes_max: u32,

    /// Max backend recv buffer bytes (per connection). Default 64 KiB.
    pub recv_buffer_bytes_max: u32,

    /// Maximum receive message reconstruction bytes in memory
    /// (across entire endpoint). Default 512 MiB.
    pub incoming_message_bytes_max: u32,

    /// Maximum size of an individual message. Default 16 MiB.
    pub message_size_max: u32,

    /// Internal event channel size. Default is 1024.
    pub internal_event_channel_size: u32,

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

    /// The backend connection module to use.
    /// For the most part you should just leave this at the default.
    pub backend_module: crate::backend::BackendModule,

    /// The backend module config to use.
    /// For the most part you should just leave this set at `None`,
    /// to get the default backend config.
    pub backend_module_config: Option<serde_json::Value>,

    /// Test falling back by failing webrtc setup.
    #[cfg(any(test, feature = "test-utils"))]
    pub test_fail_webrtc: bool,
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
            backoff_start: std::time::Duration::from_secs(5),
            backoff_max: std::time::Duration::from_secs(60),
            preflight: None,
            backend_module: BackendModule::default(),
            backend_module_config: None,
            #[cfg(any(test, feature = "test-utils"))]
            test_fail_webrtc: false,
        }
    }
}
