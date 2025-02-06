use std::sync::Arc;

/// The backend webrtc module to use.
#[derive(Debug, Clone, Copy)]
pub enum BackendModule {
    /// Use the libdatachannel backend.
    #[cfg(any(
        feature = "backend-libdatachannel",
        feature = "backend-libdatachannel-openssl"
    ))]
    LibDataChannel,

    /// Use the go pion backend.
    #[cfg(feature = "backend-go-pion")]
    GoPion,
}

impl Default for BackendModule {
    #[allow(unreachable_code)]
    fn default() -> Self {
        #[cfg(any(
            feature = "backend-libdatachannel",
            feature = "backend-libdatachannel-openssl"
        ))]
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
}
