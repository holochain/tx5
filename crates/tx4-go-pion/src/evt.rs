use crate::*;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use tx4_go_pion_sys::Event as SysEvent;
use tx4_go_pion_sys::API;

/// Incoming events related to a PeerConnection.
#[derive(Debug)]
pub enum PeerConnectionEvent {
    /// Received a trickle ICE candidate.
    ICECandidate(GoBuf),

    /// Received an incoming data channel.
    DataChannel(DataChannelSeed),
}

/// Incoming events related to a DataChannel.
#[derive(Debug)]
pub enum DataChannelEvent {
    /// DataChannel is ready to send / receive.
    Open,

    /// DataChannel is closed.
    Close,

    /// DataChannel incoming message.
    Message(GoBuf),
}

pub(crate) fn register_peer_con_evt_cb(id: usize, cb: PeerConEvtCb) {
    MANAGER.lock().peer_con.insert(id, cb);
}

pub(crate) fn unregister_peer_con_evt_cb(id: usize) {
    MANAGER.lock().peer_con.remove(&id);
}

pub(crate) fn register_data_chan_evt_cb(id: usize, cb: DataChanEvtCb) {
    MANAGER.lock().data_chan.insert(id, cb);
}

pub(crate) fn unregister_data_chan_evt_cb(id: usize) {
    MANAGER.lock().data_chan.remove(&id);
}

pub(crate) type PeerConEvtCb =
    Arc<dyn Fn(PeerConnectionEvent) + 'static + Send + Sync>;

pub(crate) type DataChanEvtCb =
    Arc<dyn Fn(DataChannelEvent) + 'static + Send + Sync>;

static MANAGER: Lazy<Mutex<Manager>> = Lazy::new(|| {
    unsafe {
        // TODO!!! MEORY LEAK
        // we need else cases througout here, if there isn't a
        // registered callback, we need to call _free functions
        // on incoming events like DataChannel and OnMessage buffers.
        API.on_event(|sys_evt| match sys_evt {
            SysEvent::Error(_error) => (),
            SysEvent::PeerConICECandidate {
                peer_con_id,
                candidate,
            } => {
                let maybe_cb =
                    MANAGER.lock().peer_con.get(&peer_con_id).cloned();
                if let Some(cb) = maybe_cb {
                    cb(PeerConnectionEvent::ICECandidate(GoBuf(candidate)));
                }
            }
            SysEvent::PeerConStateChange {
                peer_con_id: _,
                peer_con_state: _,
            } => (),
            SysEvent::PeerConDataChan {
                peer_con_id,
                data_chan_id,
            } => {
                let maybe_cb =
                    MANAGER.lock().peer_con.get(&peer_con_id).cloned();
                if let Some(cb) = maybe_cb {
                    cb(PeerConnectionEvent::DataChannel(DataChannelSeed(
                        data_chan_id,
                    )))
                }
            }
            SysEvent::DataChanClose(data_chan_id) => {
                let maybe_cb =
                    MANAGER.lock().data_chan.get(&data_chan_id).cloned();
                if let Some(cb) = maybe_cb {
                    cb(DataChannelEvent::Close)
                }
            }
            SysEvent::DataChanOpen(data_chan_id) => {
                let maybe_cb =
                    MANAGER.lock().data_chan.get(&data_chan_id).cloned();
                if let Some(cb) = maybe_cb {
                    cb(DataChannelEvent::Open)
                }
            }
            SysEvent::DataChanMessage {
                data_chan_id,
                buffer_id,
            } => {
                let maybe_cb =
                    MANAGER.lock().data_chan.get(&data_chan_id).cloned();
                if let Some(cb) = maybe_cb {
                    cb(DataChannelEvent::Message(GoBuf(buffer_id)))
                }
            }
        });
    }
    Manager::new()
});

struct Manager {
    peer_con: HashMap<usize, PeerConEvtCb>,
    data_chan: HashMap<usize, DataChanEvtCb>,
}

impl Manager {
    pub fn new() -> Mutex<Self> {
        Mutex::new(Self {
            peer_con: HashMap::new(),
            data_chan: HashMap::new(),
        })
    }
}
