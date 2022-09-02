use crate::*;
use std::sync::Arc;
use tx4_go_pion_sys::API;

/// ICE server configuration.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(crate = "tx4_core::deps::serde", rename_all = "camelCase")]
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
pub struct PeerConnectionConfig {
    /// ICE server list.
    pub ice_servers: Vec<IceServer>,
}

impl From<PeerConnectionConfig> for GoBufRef<'static> {
    fn from(p: PeerConnectionConfig) -> Self {
        GoBufRef::json(&p)
    }
}

impl From<&PeerConnectionConfig> for GoBufRef<'static> {
    fn from(p: &PeerConnectionConfig) -> Self {
        GoBufRef::json(&p)
    }
}

/// Configuration for a go pion webrtc DataChannel.
#[derive(Default, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(crate = "tx4_core::deps::serde", rename_all = "camelCase")]
pub struct DataChannelConfig {
    /// DataChannel Label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

impl From<DataChannelConfig> for GoBufRef<'static> {
    fn from(p: DataChannelConfig) -> Self {
        GoBufRef::json(&p)
    }
}

impl From<&DataChannelConfig> for GoBufRef<'static> {
    fn from(p: &DataChannelConfig) -> Self {
        GoBufRef::json(&p)
    }
}

/// Configuration for a go pion webrtc PeerConnection offer.
#[derive(Default, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(crate = "tx4_core::deps::serde", rename_all = "camelCase")]
pub struct OfferConfig {}

impl From<OfferConfig> for GoBufRef<'static> {
    fn from(p: OfferConfig) -> Self {
        GoBufRef::json(&p)
    }
}

impl From<&OfferConfig> for GoBufRef<'static> {
    fn from(p: &OfferConfig) -> Self {
        GoBufRef::json(&p)
    }
}

/// Configuration for a go pion webrtc PeerConnection answer.
#[derive(Default, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(crate = "tx4_core::deps::serde", rename_all = "camelCase")]
pub struct AnswerConfig {}

impl From<AnswerConfig> for GoBufRef<'static> {
    fn from(p: AnswerConfig) -> Self {
        GoBufRef::json(&p)
    }
}

impl From<&AnswerConfig> for GoBufRef<'static> {
    fn from(p: &AnswerConfig) -> Self {
        GoBufRef::json(&p)
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
    pub fn new<'a, B, Cb>(config: B, cb: Cb) -> Result<Self>
    where
        B: Into<GoBufRef<'a>>,
        Cb: Fn(PeerConnectionEvent) + 'static + Send + Sync,
    {
        init_evt_manager();
        let mut config = config.into();
        let config = config.as_mut_ref()?;
        let cb: PeerConEvtCb = Arc::new(cb);
        unsafe {
            let peer_con_id = API.peer_con_alloc(config.0)?;
            register_peer_con_evt_cb(peer_con_id, cb);
            Ok(Self(peer_con_id))
        }
    }

    /// Create offer.
    pub fn create_offer<'a, B>(&mut self, config: B) -> Result<GoBuf>
    where
        B: Into<GoBufRef<'a>>,
    {
        let mut config = config.into();
        let config = config.as_mut_ref()?;
        unsafe { API.peer_con_create_offer(self.0, config.0).map(GoBuf) }
    }

    /// Create answer.
    pub fn create_answer<'a, B>(&mut self, config: B) -> Result<GoBuf>
    where
        B: Into<GoBufRef<'a>>,
    {
        let mut config = config.into();
        let config = config.as_mut_ref()?;
        unsafe { API.peer_con_create_answer(self.0, config.0).map(GoBuf) }
    }

    /// Set local description.
    pub fn set_local_description<'a, B>(&mut self, desc: B) -> Result<()>
    where
        B: Into<GoBufRef<'a>>,
    {
        let mut desc = desc.into();
        let desc = desc.as_mut_ref()?;
        unsafe { API.peer_con_set_local_desc(self.0, desc.0) }
    }

    /// Set remote description.
    pub fn set_remote_description<'a, B>(&mut self, desc: B) -> Result<()>
    where
        B: Into<GoBufRef<'a>>,
    {
        let mut desc = desc.into();
        let desc = desc.as_mut_ref()?;
        unsafe { API.peer_con_set_rem_desc(self.0, desc.0) }
    }

    /// Add ice candidate.
    pub fn add_ice_candidate<'a, B>(&mut self, ice: B) -> Result<()>
    where
        B: Into<GoBufRef<'a>>,
    {
        let mut ice = ice.into();
        let ice = ice.as_mut_ref()?;
        unsafe { API.peer_con_add_ice_candidate(self.0, ice.0) }
    }

    /// Create data channel.
    pub fn create_data_channel<'a, B>(
        &mut self,
        config: B,
    ) -> Result<DataChannelSeed>
    where
        B: Into<GoBufRef<'a>>,
    {
        let mut config = config.into();
        let config = config.as_mut_ref()?;
        unsafe {
            let data_chan_id =
                API.peer_con_create_data_chan(self.0, config.0)?;
            Ok(DataChannelSeed(data_chan_id))
        }
    }
}
