use crate::*;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use tx5_go_pion_sys::Event as SysEvent;
use tx5_go_pion_sys::API;

/// PeerConnectionState events.
#[derive(Debug)]
pub enum PeerConnectionState {
    /// <https://pkg.go.dev/github.com/pion/webrtc/v3#PeerConnectionState>
    New = 0x01,

    /// <https://pkg.go.dev/github.com/pion/webrtc/v3#PeerConnectionState>
    Connecting = 0x02,

    /// <https://pkg.go.dev/github.com/pion/webrtc/v3#PeerConnectionState>
    Connected = 0x03,

    /// <https://pkg.go.dev/github.com/pion/webrtc/v3#PeerConnectionState>
    Disconnected = 0x04,

    /// <https://pkg.go.dev/github.com/pion/webrtc/v3#PeerConnectionState>
    Failed = 0x05,

    /// <https://pkg.go.dev/github.com/pion/webrtc/v3#PeerConnectionState>
    Closed = 0x06,
}

impl PeerConnectionState {
    /// Construct from a raw integer value.
    pub fn from_raw(raw: usize) -> Self {
        match raw {
            0x01 => PeerConnectionState::New,
            0x02 => PeerConnectionState::Connecting,
            0x03 => PeerConnectionState::Connected,
            0x04 => PeerConnectionState::Disconnected,
            0x05 => PeerConnectionState::Failed,
            0x06 => PeerConnectionState::Closed,
            _ => panic!("invalid raw PeerConnectionState value: {raw}"),
        }
    }
}

/// Incoming events related to a PeerConnection.
#[derive(Debug)]
pub enum PeerConnectionEvent {
    /// PeerConnection Error.
    Error(std::io::Error),

    /// PeerConnectionState event.
    State(PeerConnectionState),

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

    /// DataChannel buffered amount is now low.
    BufferedAmountLow,
}

#[inline]
pub(crate) fn init_evt_manager() {
    // ensure initialization
    MANAGER.is_locked();
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

pub(crate) fn replace_data_chan_evt_cb<F>(id: usize, f: F)
where
    F: FnOnce() -> DataChanEvtCb,
{
    let mut lock = MANAGER.lock();
    let cb = f();
    lock.data_chan.insert(id, cb);
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
                peer_con_id,
                peer_con_state,
            } => {
                let maybe_cb =
                    MANAGER.lock().peer_con.get(&peer_con_id).cloned();
                if let Some(cb) = maybe_cb {
                    cb(PeerConnectionEvent::State(
                        PeerConnectionState::from_raw(peer_con_state),
                    ));
                }
            }
            SysEvent::PeerConDataChan {
                peer_con_id,
                data_chan_id,
            } => {
                let maybe_cb =
                    MANAGER.lock().peer_con.get(&peer_con_id).cloned();
                if let Some(cb) = maybe_cb {
                    cb(PeerConnectionEvent::DataChannel(DataChannelSeed::new(
                        data_chan_id,
                    )));
                }
            }
            SysEvent::DataChanClose(data_chan_id) => {
                let maybe_cb =
                    MANAGER.lock().data_chan.get(&data_chan_id).cloned();
                if let Some(cb) = maybe_cb {
                    cb(DataChannelEvent::Close);
                }
            }
            SysEvent::DataChanOpen(data_chan_id) => {
                let maybe_cb =
                    MANAGER.lock().data_chan.get(&data_chan_id).cloned();
                if let Some(cb) = maybe_cb {
                    cb(DataChannelEvent::Open);
                }
            }
            SysEvent::DataChanMessage {
                data_chan_id,
                buffer_id,
            } => {
                let maybe_cb =
                    MANAGER.lock().data_chan.get(&data_chan_id).cloned();
                if let Some(cb) = maybe_cb {
                    cb(DataChannelEvent::Message(GoBuf(buffer_id)));
                }
            }
            SysEvent::DataChanBufferedAmountLow(data_chan_id) => {
                let maybe_cb =
                    MANAGER.lock().data_chan.get(&data_chan_id).cloned();
                if let Some(cb) = maybe_cb {
                    cb(DataChannelEvent::BufferedAmountLow);
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
