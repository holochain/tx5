use crate::*;
use std::sync::{Arc, Mutex, Weak};
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

pub(crate) struct PeerConCore {
    peer_con_id: usize,
    evt_send: tokio::sync::mpsc::UnboundedSender<PeerConnectionEvent>,
    drop_err: Error,
}

impl Drop for PeerConCore {
    fn drop(&mut self) {
        let _ = self
            .evt_send
            .send(PeerConnectionEvent::Error(self.drop_err.clone()));
        unregister_peer_con(self.peer_con_id);
        unsafe {
            API.peer_con_free(self.peer_con_id);
        }
    }
}

impl PeerConCore {
    pub fn new(
        peer_con_id: usize,
        evt_send: tokio::sync::mpsc::UnboundedSender<PeerConnectionEvent>,
    ) -> Self {
        Self {
            peer_con_id,
            evt_send,
            drop_err: Error::id("PeerConnectionDropped").into(),
        }
    }

    pub fn close(&mut self, err: Error) {
        // self.evt_send.send_err() is called in Drop impl
        self.drop_err = err;
    }
}

#[derive(Clone)]
pub(crate) struct WeakPeerCon(
    pub(crate) Weak<Mutex<std::result::Result<PeerConCore, Error>>>,
);

macro_rules! peer_con_strong_core {
    ($inner:expr, $ident:ident, $block:block) => {
        match &mut *$inner.lock().unwrap() {
            Ok($ident) => $block,
            Err(err) => Result::Err(err.clone().into()),
        }
    };
}

macro_rules! peer_con_weak_core {
    ($inner:expr, $ident:ident, $block:block) => {
        match $inner.upgrade() {
            Some(strong) => peer_con_strong_core!(strong, $ident, $block),
            None => Result::Err(Error::id("PeerConnectionClosed")),
        }
    };
}

impl WeakPeerCon {
    /*
    pub fn close(&self, err: Error) {
        if let Some(strong) = self.0.upgrade() {
            let mut tmp = Err(err.clone());

            {
                let mut lock = strong.lock().unwrap();
                let mut do_swap = false;
                if let Ok(core) = &mut *lock {
                    core.close(err.clone());
                    do_swap = true;
                }
                if do_swap {
                    std::mem::swap(&mut *lock, &mut tmp);
                }
            }

            // make sure the above lock is released before this is dropped
            drop(tmp);
        }
    }
    */
    pub fn send_evt(&self, evt: PeerConnectionEvent) -> Result<()> {
        peer_con_weak_core!(self.0, core, {
            core.evt_send
                .send(evt)
                .map_err(|_| Error::id("PeerConnectionClosed"))
        })
    }
}

/// A go pion webrtc PeerConnection.
pub struct PeerConnection(Arc<Mutex<std::result::Result<PeerConCore, Error>>>);

impl PeerConnection {
    /// Construct a new PeerConnection.
    /// Warning: This returns an unbounded channel,
    /// you should process this as quickly and synchronously as possible
    /// to avoid a backlog filling up memory.
    pub async fn new<'a, B>(
        config: B,
    ) -> Result<(
        Self,
        tokio::sync::mpsc::UnboundedReceiver<PeerConnectionEvent>,
    )>
    where
        B: Into<GoBufRef<'a>>,
    {
        tx5_init().await.map_err(Error::err)?;
        init_evt_manager();
        r2id!(config);
        tokio::task::spawn_blocking(move || unsafe {
            let peer_con_id = API.peer_con_alloc(config)?;
            let (evt_send, evt_recv) = tokio::sync::mpsc::unbounded_channel();

            let strong = Arc::new(Mutex::new(Ok(PeerConCore::new(
                peer_con_id,
                evt_send,
            ))));

            let weak = WeakPeerCon(Arc::downgrade(&strong));

            register_peer_con(peer_con_id, weak);

            Ok((Self(strong), evt_recv))
        })
        .await?
    }

    /// Close this connection.
    pub fn close(&self, err: Error) {
        let mut tmp = Err(err.clone());

        {
            let mut lock = self.0.lock().unwrap();
            let mut do_swap = false;
            if let Ok(core) = &mut *lock {
                core.close(err.clone());
                do_swap = true;
            }
            if do_swap {
                std::mem::swap(&mut *lock, &mut tmp);
            }
        }

        // make sure the above lock is released before this is dropped
        drop(tmp);
    }

    fn get_peer_con_id(&self) -> Result<usize> {
        peer_con_strong_core!(self.0, core, { Ok(core.peer_con_id) })
    }

    /// Get stats.
    pub async fn stats(&self) -> Result<GoBuf> {
        let peer_con_id = self.get_peer_con_id()?;

        tokio::task::spawn_blocking(move || unsafe {
            API.peer_con_stats(peer_con_id).map(GoBuf)
        })
        .await?
    }

    /// Create offer.
    pub async fn create_offer<'a, B>(&self, config: B) -> Result<GoBuf>
    where
        B: Into<GoBufRef<'a>>,
    {
        let peer_con_id = self.get_peer_con_id()?;

        r2id!(config);
        tokio::task::spawn_blocking(move || unsafe {
            API.peer_con_create_offer(peer_con_id, config).map(GoBuf)
        })
        .await?
    }

    /// Create answer.
    pub async fn create_answer<'a, B>(&self, config: B) -> Result<GoBuf>
    where
        B: Into<GoBufRef<'a>>,
    {
        let peer_con_id = self.get_peer_con_id()?;

        r2id!(config);
        tokio::task::spawn_blocking(move || unsafe {
            API.peer_con_create_answer(peer_con_id, config).map(GoBuf)
        })
        .await?
    }

    /// Set local description.
    pub async fn set_local_description<'a, B>(&self, desc: B) -> Result<()>
    where
        B: Into<GoBufRef<'a>>,
    {
        let peer_con_id = self.get_peer_con_id()?;

        r2id!(desc);
        tokio::task::spawn_blocking(move || unsafe {
            API.peer_con_set_local_desc(peer_con_id, desc)
        })
        .await?
    }

    /// Set remote description.
    pub async fn set_remote_description<'a, B>(&self, desc: B) -> Result<()>
    where
        B: Into<GoBufRef<'a>>,
    {
        let peer_con_id = self.get_peer_con_id()?;

        r2id!(desc);
        tokio::task::spawn_blocking(move || unsafe {
            API.peer_con_set_rem_desc(peer_con_id, desc)
        })
        .await?
    }

    /// Add ice candidate.
    pub async fn add_ice_candidate<'a, B>(&self, ice: B) -> Result<()>
    where
        B: Into<GoBufRef<'a>>,
    {
        let peer_con_id = self.get_peer_con_id()?;

        r2id!(ice);
        tokio::task::spawn_blocking(move || unsafe {
            API.peer_con_add_ice_candidate(peer_con_id, ice)
        })
        .await?
    }

    /// Create data channel.
    pub async fn create_data_channel<'a, B>(
        &self,
        config: B,
    ) -> Result<(
        DataChannel,
        tokio::sync::mpsc::UnboundedReceiver<DataChannelEvent>,
    )>
    where
        B: Into<GoBufRef<'a>>,
    {
        let peer_con_id =
            peer_con_strong_core!(self.0, core, { Ok(core.peer_con_id) })?;

        r2id!(config);
        tokio::task::spawn_blocking(move || unsafe {
            let data_chan_id =
                API.peer_con_create_data_chan(peer_con_id, config)?;
            Ok(DataChannel::new(data_chan_id))
        })
        .await?
    }
}
