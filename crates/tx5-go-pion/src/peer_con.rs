use crate::*;
use std::sync::Arc;
use tx5_go_pion_sys::API;

/// ICE server configuration.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(crate = "tx5_core::deps::serde", rename_all = "camelCase")]
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
#[serde(crate = "tx5_core::deps::serde", rename_all = "camelCase")]
pub struct PeerConnectionConfig {
    /// ICE server list.
    pub ice_servers: Vec<IceServer>,
}

impl From<PeerConnectionConfig> for GoBufRef<'static> {
    fn from(p: PeerConnectionConfig) -> Self {
        GoBufRef::json(p)
    }
}

impl From<&PeerConnectionConfig> for GoBufRef<'static> {
    fn from(p: &PeerConnectionConfig) -> Self {
        GoBufRef::json(p)
    }
}

/// Configuration for a go pion webrtc DataChannel.
#[derive(Default, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(crate = "tx5_core::deps::serde", rename_all = "camelCase")]
pub struct DataChannelConfig {
    /// DataChannel Label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

impl From<DataChannelConfig> for GoBufRef<'static> {
    fn from(p: DataChannelConfig) -> Self {
        GoBufRef::json(p)
    }
}

impl From<&DataChannelConfig> for GoBufRef<'static> {
    fn from(p: &DataChannelConfig) -> Self {
        GoBufRef::json(p)
    }
}

/// Configuration for a go pion webrtc PeerConnection offer.
#[derive(Default, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(crate = "tx5_core::deps::serde", rename_all = "camelCase")]
pub struct OfferConfig {}

impl From<OfferConfig> for GoBufRef<'static> {
    fn from(p: OfferConfig) -> Self {
        GoBufRef::json(p)
    }
}

impl From<&OfferConfig> for GoBufRef<'static> {
    fn from(p: &OfferConfig) -> Self {
        GoBufRef::json(p)
    }
}

/// Configuration for a go pion webrtc PeerConnection answer.
#[derive(Default, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(crate = "tx5_core::deps::serde", rename_all = "camelCase")]
pub struct AnswerConfig {}

impl From<AnswerConfig> for GoBufRef<'static> {
    fn from(p: AnswerConfig) -> Self {
        GoBufRef::json(p)
    }
}

impl From<&AnswerConfig> for GoBufRef<'static> {
    fn from(p: &AnswerConfig) -> Self {
        GoBufRef::json(p)
    }
}

/// A go pion webrtc PeerConnection.
#[derive(Debug)]
pub struct PeerConnection(usize);

impl Drop for PeerConnection {
    fn drop(&mut self) {
        unsafe {
            unregister_peer_con_evt_cb(self.0);
            API.peer_con_free(self.0);
        }
    }
}

impl PeerConnection {
    /// Construct a new PeerConnection.
    pub async fn new<'a, B, Cb>(
        peer_con_config: B,
        tx5_init_config: Tx5InitConfig,
        cb: Cb,
    ) -> Result<Self>
    where
        B: Into<GoBufRef<'a>>,
        Cb: Fn(PeerConnectionEvent) + 'static + Send + Sync,
    {
        tx5_init(tx5_init_config).await?;
        init_evt_manager();
        r2id!(peer_con_config);
        let cb: PeerConEvtCb = Arc::new(cb);
        tokio::task::spawn_blocking(move || unsafe {
            let peer_con_id = API.peer_con_alloc(peer_con_config)?;
            register_peer_con_evt_cb(peer_con_id, cb);
            Ok(Self(peer_con_id))
        })
        .await?
    }

    /// Get stats.
    pub async fn stats(&mut self) -> Result<GoBuf> {
        let peer_con = self.0;
        tokio::task::spawn_blocking(move || unsafe {
            API.peer_con_stats(peer_con).map(GoBuf)
        })
        .await?
    }

    /// Create offer.
    pub async fn create_offer<'a, B>(&mut self, config: B) -> Result<GoBuf>
    where
        B: Into<GoBufRef<'a>>,
    {
        let peer_con = self.0;
        r2id!(config);
        tokio::task::spawn_blocking(move || unsafe {
            API.peer_con_create_offer(peer_con, config).map(GoBuf)
        })
        .await?
    }

    /// Create answer.
    pub async fn create_answer<'a, B>(&mut self, config: B) -> Result<GoBuf>
    where
        B: Into<GoBufRef<'a>>,
    {
        let peer_con = self.0;
        r2id!(config);
        tokio::task::spawn_blocking(move || unsafe {
            API.peer_con_create_answer(peer_con, config).map(GoBuf)
        })
        .await?
    }

    /// Set local description.
    pub async fn set_local_description<'a, B>(&mut self, desc: B) -> Result<()>
    where
        B: Into<GoBufRef<'a>>,
    {
        let peer_con = self.0;
        r2id!(desc);
        tokio::task::spawn_blocking(move || unsafe {
            API.peer_con_set_local_desc(peer_con, desc)
        })
        .await?
    }

    /// Set remote description.
    pub async fn set_remote_description<'a, B>(&mut self, desc: B) -> Result<()>
    where
        B: Into<GoBufRef<'a>>,
    {
        let peer_con = self.0;
        r2id!(desc);
        tokio::task::spawn_blocking(move || unsafe {
            API.peer_con_set_rem_desc(peer_con, desc)
        })
        .await?
    }

    /// Add ice candidate.
    pub async fn add_ice_candidate<'a, B>(&mut self, ice: B) -> Result<()>
    where
        B: Into<GoBufRef<'a>>,
    {
        let peer_con = self.0;
        r2id!(ice);
        tokio::task::spawn_blocking(move || unsafe {
            API.peer_con_add_ice_candidate(peer_con, ice)
        })
        .await?
    }

    /// Create data channel.
    pub async fn create_data_channel<'a, B>(
        &mut self,
        config: B,
    ) -> Result<DataChannelSeed>
    where
        B: Into<GoBufRef<'a>>,
    {
        let peer_con = self.0;
        r2id!(config);
        tokio::task::spawn_blocking(move || unsafe {
            let data_chan_id =
                API.peer_con_create_data_chan(peer_con, config)?;
            Ok(DataChannelSeed::new(data_chan_id))
        })
        .await?
    }
}
