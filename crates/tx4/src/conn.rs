//! Tx4 peer connection abstractions.

use crate::*;

#[cfg(feature = "backend-go-pion")]
mod imp {
    mod imp_go_pion;
    pub use imp_go_pion::*;
}

/// Events emitted by a PeerConnection.
#[derive(Debug)]
pub enum PeerConnectionEvent {
    /// Ice candidate event.
    IceCandidate(Buf),

    /// Incoming data channel seed.
    DataChannel(DataChannelSeed),
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

/// Configuration for PeerConnection::create_answer.
#[derive(Default, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(crate = "tx4_core::deps::serde", rename_all = "camelCase")]
#[non_exhaustive]
pub struct AnswerConfig {}

impl AsRef<AnswerConfig> for AnswerConfig {
    fn as_ref(&self) -> &AnswerConfig {
        self
    }
}

/// Configuration for a go pion webrtc DataChannel.
#[derive(Default, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(crate = "tx4_core::deps::serde", rename_all = "camelCase")]
#[non_exhaustive]
pub struct DataChannelConfig {
    /// DataChannel Label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

impl DataChannelConfig {
    /// Set the label for this DataChannelConfig
    pub fn with_label(mut self, label: impl std::fmt::Display) -> Self {
        self.label = Some(label.to_string());
        self
    }
}

impl AsRef<DataChannelConfig> for DataChannelConfig {
    fn as_ref(&self) -> &DataChannelConfig {
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

    /// Create an "answer" for the remote side of this PeerConnection.
    ///
    /// # Example
    ///
    /// ```
    /// # use tx4::*;
    /// # #[tokio::main]
    /// # async fn main() {
    /// # let mut conn1 = PeerConnection::new(
    /// #     PeerConnectionConfig::default(),
    /// #     |_evt| {},
    /// # ).await.unwrap();
    /// # conn1.create_data_channel(
    /// #     DataChannelConfig::default().with_label("data"),
    /// # ).await.unwrap();
    /// # let mut offer =
    /// #     conn1.create_offer(OfferConfig::default()).await.unwrap();
    /// # conn1.set_local_description(&mut offer).await.unwrap();
    /// # let mut conn2 = PeerConnection::new(
    /// #     PeerConnectionConfig::default(),
    /// #     |_evt| {},
    /// # ).await.unwrap();
    /// # conn2.set_remote_description(&mut offer).await.unwrap();
    /// let answer = conn2.create_answer(AnswerConfig::default()).await.unwrap();
    /// # std::mem::drop(answer);
    /// # }
    /// ```
    pub async fn create_answer<'a, B>(&mut self, config: B) -> Result<Buf>
    where
        B: Into<BufRef<'a>>,
    {
        self.imp.create_answer(config).await
    }

    /// Set the local description to the appropriate offer or answer.
    ///
    /// # Example
    ///
    /// ```
    /// # use tx4::*;
    /// # #[tokio::main]
    /// # async fn main() {
    /// # let mut conn1 = PeerConnection::new(
    /// #     PeerConnectionConfig::default(),
    /// #     |_evt| {},
    /// # ).await.unwrap();
    /// # conn1.create_data_channel(
    /// #     DataChannelConfig::default().with_label("data"),
    /// # ).await.unwrap();
    /// # let mut offer =
    /// #     conn1.create_offer(OfferConfig::default()).await.unwrap();
    /// conn1.set_local_description(&mut offer).await.unwrap();
    /// # }
    /// ```
    pub async fn set_local_description<'a, B>(&mut self, desc: B) -> Result<()>
    where
        B: Into<BufRef<'a>>,
    {
        self.imp.set_local_description(desc).await
    }

    /// Set the remote description to the appropriate offer or answer.
    ///
    /// # Example
    ///
    /// ```
    /// # use tx4::*;
    /// # #[tokio::main]
    /// # async fn main() {
    /// # let mut conn1 = PeerConnection::new(
    /// #     PeerConnectionConfig::default(),
    /// #     |_evt| {},
    /// # ).await.unwrap();
    /// # conn1.create_data_channel(
    /// #     DataChannelConfig::default().with_label("data"),
    /// # ).await.unwrap();
    /// # let mut offer =
    /// #     conn1.create_offer(OfferConfig::default()).await.unwrap();
    /// # conn1.set_local_description(&mut offer).await.unwrap();
    /// # let mut conn2 = PeerConnection::new(
    /// #     PeerConnectionConfig::default(),
    /// #     |_evt| {},
    /// # ).await.unwrap();
    /// conn2.set_remote_description(&mut offer).await.unwrap();
    /// # }
    /// ```
    pub async fn set_remote_description<'a, B>(&mut self, desc: B) -> Result<()>
    where
        B: Into<BufRef<'a>>,
    {
        self.imp.set_remote_description(desc).await
    }

    /// yo
    pub async fn add_ice_candidate<'a, B>(&mut self, ice: B) -> Result<()>
    where
        B: Into<BufRef<'a>>,
    {
        self.imp.add_ice_candidate(ice).await
    }

    /// Trigger a data channel to be created. The data channel (when ready)
    /// will be emitted via PeerConnectionEvent::DataChannel.
    ///
    /// # Example
    ///
    /// ```
    /// # use tx4::*;
    /// # #[tokio::main]
    /// # async fn main() {
    /// # let mut conn1 = PeerConnection::new(
    /// #     PeerConnectionConfig::default(),
    /// #     |_evt| {},
    /// # ).await.unwrap();
    /// conn1.create_data_channel(
    ///     DataChannelConfig::default().with_label("data"),
    /// ).await.unwrap();
    /// # }
    /// ```
    pub async fn create_data_channel<'a, B>(
        &mut self,
        config: B,
    ) -> Result<DataChannelSeed>
    where
        B: Into<BufRef<'a>>,
    {
        self.imp.create_data_channel(config).await
    }
}
