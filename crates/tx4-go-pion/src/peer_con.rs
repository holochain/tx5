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
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(crate = "tx4_core::deps::serde", rename_all = "camelCase")]
pub struct PeerConConfig {
    /// ICE server list.
    pub ice_servers: Vec<IceServer>,
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
    pub fn new<B, Cb>(config: B, cb: Cb) -> Result<Self>
    where
        B: Into<IntoGoBuf>,
        Cb: Fn(PeerConnectionEvent) + 'static + Send + Sync,
    {
        let config: Result<GoBuf> = config.into().into();
        let cb: PeerConEvtCb = Arc::new(cb);
        unsafe {
            let config = config?;
            let peer_con_id = API.peer_con_alloc(config.0)?;
            register_peer_con_evt_cb(peer_con_id, cb);
            Ok(Self(peer_con_id))
        }
    }

    /// Create offer.
    pub fn create_offer(&mut self, json: Option<&str>) -> Result<String> {
        unsafe { API.peer_con_create_offer(self.0, json) }
    }

    /// Create answer.
    pub fn create_answer(&mut self, json: Option<&str>) -> Result<String> {
        unsafe { API.peer_con_create_answer(self.0, json) }
    }

    /// Set local description.
    pub fn set_local_description(&mut self, json: &str) -> Result<()> {
        unsafe { API.peer_con_set_local_desc(self.0, json) }
    }

    /// Set remote description.
    pub fn set_remote_description(&mut self, json: &str) -> Result<()> {
        unsafe { API.peer_con_set_rem_desc(self.0, json) }
    }

    /// Add ice candidate.
    pub fn add_ice_candidate(&mut self, json: &str) -> Result<()> {
        unsafe { API.peer_con_add_ice_candidate(self.0, json) }
    }

    /// Create data channel.
    pub fn create_data_channel(
        &mut self,
        json: &str,
    ) -> Result<DataChannelSeed> {
        unsafe {
            let data_chan_id = API.peer_con_create_data_chan(self.0, json)?;
            Ok(DataChannelSeed(data_chan_id))
        }
    }
}
