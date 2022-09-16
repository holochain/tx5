//! Tx4 peer connection abstractions.

use crate::*;

#[cfg(feature = "backend-go-pion")]
mod imp {
    mod imp_go_pion;
    pub use imp_go_pion::*;
}

/// Events emitted by a PeerConnection.
pub enum PeerConnectionEvent {
    /// Ice candidate event.
    IceCandidate(Buf),
}

/// ICE server configuration.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(crate = "tx4_core::deps::serde", rename_all = "camelCase")]
#[non_exhaustive]
pub struct IceServer {
    /// Url list.
    pub urls: Vec<String>,

    /// Optional username.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,

    /// Optional credential.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
}

/// Configuration for a go pion webrtc PeerConnection.
#[derive(Default, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(crate = "tx4_core::deps::serde", rename_all = "camelCase")]
#[non_exhaustive]
pub struct PeerConnectionConfig {
    /// Ice server list.
    pub ice_servers: Vec<IceServer>,
}

/// Configuration for PeerConnection::create_offer.
#[derive(Default, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(crate = "tx4_core::deps::serde", rename_all = "camelCase")]
#[non_exhaustive]
pub struct OfferConfig {}

impl AsRef<OfferConfig> for OfferConfig {
    fn as_ref(&self) -> &OfferConfig {
        self
    }
}

/// Tx4 peer connection.
pub struct PeerConnection {
    imp: imp::ImpConn,
    _not_sync: std::marker::PhantomData<std::cell::Cell<()>>,
}

impl PeerConnection {
    /// Construct a new PeerConnection.
    ///
    /// # Example
    ///
    /// ```
    /// # use tx4::*;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let conn = PeerConnection::new(
    ///     PeerConnectionConfig::default(),
    ///     |_evt| {},
    /// ).await.unwrap();
    /// # std::mem::drop(conn);
    /// # }
    /// ```
    pub async fn new<'a, B, Cb>(config: B, cb: Cb) -> Result<Self>
    where
        B: Into<BufRef<'a>>,
        Cb: Fn(PeerConnectionEvent) + 'static + Send + Sync,
    {
        Ok(Self {
            imp: imp::ImpConn::new(config, cb).await?,
            _not_sync: std::marker::PhantomData,
        })
    }

    /// Create an "offer" for the remote side of this PeerConnection.
    ///
    /// # Example
    ///
    /// ```
    /// # use tx4::*;
    /// # #[tokio::main]
    /// # async fn main() {
    /// # let mut conn = PeerConnection::new(
    /// #     PeerConnectionConfig::default(),
    /// #     |_evt| {},
    /// # ).await.unwrap();
    /// let offer = conn.create_offer(OfferConfig::default()).await.unwrap();
    /// # std::mem::drop(offer);
    /// # }
    /// ```
    pub async fn create_offer<'a, B>(&mut self, config: B) -> Result<Buf>
    where
        B: Into<BufRef<'a>>,
    {
        self.imp.create_offer(config).await
    }
}
