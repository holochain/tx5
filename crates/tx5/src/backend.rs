//! Backend modules usable by tx5.

use std::io::Result;
use std::sync::Arc;

use futures::future::BoxFuture;

use crate::{Config, PubKey};
use tx5_core::deps::serde_json;

#[cfg(any(feature = "backend-go-pion", feature = "backend-libdatachannel"))]
mod be_tx5_connection;

mod mem;

/// Backend modules usable by tx5.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BackendModule {
    #[cfg(feature = "backend-libdatachannel")]
    /// The libdatachannel-based backend.
    LibDataChannel,

    #[cfg(feature = "backend-go-pion")]
    /// The Go Pion-based backend.
    GoPion,

    #[cfg(feature = "backend-webrtc-rs")]
    /// The Webrtc-RS-based backend.
    WebrtcRs,

    /// The mem backend.
    Mem,
}

impl Default for BackendModule {
    #[allow(unreachable_code)]
    fn default() -> Self {
        #[cfg(feature = "backend-libdatachannel")]
        return Self::LibDataChannel;
        #[cfg(feature = "backend-go-pion")]
        return Self::GoPion;
        #[cfg(feature = "backend-webrtc-rs")]
        return Self::WebrtcRs;
        Self::Mem
    }
}

impl BackendModule {
    /// Get a default version of the module-specific config.
    pub fn default_config(&self) -> serde_json::Value {
        match self {
            #[cfg(feature = "backend-libdatachannel")]
            Self::LibDataChannel => be_tx5_connection::default_config(),
            #[cfg(feature = "backend-go-pion")]
            Self::GoPion => be_tx5_connection::default_config(),
            #[cfg(feature = "backend-webrtc-rs")]
            Self::WebrtcRs => todo!(),
            Self::Mem => mem::default_config(),
        }
    }

    /// Connect a new backend module endpoint.
    pub async fn connect(
        &self,
        url: &str,
        listener: bool,
        config: &Arc<Config>,
    ) -> Result<(DynBackEp, DynBackEpRecvCon)> {
        match self {
            #[cfg(feature = "backend-libdatachannel")]
            Self::LibDataChannel => {
                be_tx5_connection::connect(config, url, listener).await
            }
            #[cfg(feature = "backend-go-pion")]
            Self::GoPion => {
                be_tx5_connection::connect(config, url, listener).await
            }
            #[cfg(feature = "backend-webrtc-rs")]
            Self::WebrtcRs => todo!(),
            Self::Mem => mem::connect(config, url, listener).await,
        }
    }
}

/// Backend connection.
pub trait BackCon: 'static + Send + Sync {
    /// Send data over this backend connection.
    fn send(&self, data: Vec<u8>) -> BoxFuture<'_, Result<()>>;

    /// Get the pub_key identifying this connection.
    fn pub_key(&self) -> &PubKey;

    /// Returns `true` if we successfully connected over webrtc.
    // TODO - this isn't good encapsulation
    fn is_using_webrtc(&self) -> bool;

    /// Get connection statistics.
    // TODO - this isn't good encapsulation
    fn get_stats(&self) -> tx5_connection::ConnStats;
}

/// Trait-object version of backend connection.
pub type DynBackCon = Arc<dyn BackCon + 'static + Send + Sync>;

/// Backend connection receiver.
pub trait BackConRecvData: 'static + Send {
    /// Receive data from this backend connection.
    fn recv(&mut self) -> BoxFuture<'_, Option<Vec<u8>>>;
}

/// Trait-object version of backend connection receiver.
pub type DynBackConRecvData = Box<dyn BackConRecvData + 'static + Send>;

/// Pending connection.
pub trait BackWaitCon: 'static + Send {
    /// Wait for the connection
    fn wait(
        &mut self,
        // TODO - this isn't good encapsulation
        recv_limit: Arc<tokio::sync::Semaphore>,
    ) -> BoxFuture<'static, Result<(DynBackCon, DynBackConRecvData)>>;

    /// Get the pub_key identifying this connection.
    fn pub_key(&self) -> &PubKey;
}

/// Trait-object version of backend wait con.
pub type DynBackWaitCon = Box<dyn BackWaitCon + 'static + Send>;

/// Backend endpoint.
pub trait BackEp: 'static + Send + Sync {
    /// Establish an outgoing connection from this backend endpoint.
    fn connect(&self, pub_key: PubKey)
        -> BoxFuture<'_, Result<DynBackWaitCon>>;

    /// Get the pub_key identifying this endpoint.
    fn pub_key(&self) -> &PubKey;
}

/// Trait-object version of backend endpoint.
pub type DynBackEp = Arc<dyn BackEp + 'static + Send + Sync>;

/// Backend endpoint receiver.
pub trait BackEpRecvCon: 'static + Send {
    /// Receive incoming connection from this backend endpoint.
    fn recv(&mut self) -> BoxFuture<'_, Option<DynBackWaitCon>>;
}

/// Trait-object version of backend endpoint receiver.
pub type DynBackEpRecvCon = Box<dyn BackEpRecvCon + 'static + Send>;
