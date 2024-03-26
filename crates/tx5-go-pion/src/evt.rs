use crate::*;
use once_cell::sync::Lazy;
use std::collections::HashMap;
use tx5_go_pion_sys::Event as SysEvent;
use tx5_go_pion_sys::API;

/// PeerConnectionState events.
#[derive(Debug, Clone, Copy)]
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
    /// PeerConnection error.
    Error(Error),

    /// PeerConnectionState event.
    State(PeerConnectionState),

    /// Received a trickle ICE candidate.
    ICECandidate(GoBuf),

    /// Received an incoming data channel.
    /// Warning: This returns an unbounded channel,
    /// you should process this as quickly and synchronously as possible
    /// to avoid a backlog filling up memory.
    DataChannel(
        DataChannel,
        tokio::sync::mpsc::UnboundedReceiver<DataChannelEvent>,
    ),
}

impl From<Error> for PeerConnectionEvent {
    fn from(err: Error) -> Self {
        Self::Error(err)
    }
}

/// Incoming events related to a DataChannel.
#[derive(Debug)]
pub enum DataChannelEvent {
    /// Data channel error.
    Error(Error),

    /// Data channel has transitioned to "open".
    Open,

    /// Data channel has transitioned to "closed".
    Close,

    /// Received incoming message on the data channel.
    Message(GoBuf),

    /// Data channel buffered amount is now low.
    BufferedAmountLow,
}

impl From<Error> for DataChannelEvent {
    fn from(err: Error) -> Self {
        Self::Error(err)
    }
}

#[inline]
pub(crate) fn init_evt_manager() {
    // ensure initialization
    let _ = &*MANAGER;
}

enum Cmd {
    Evt(SysEvent),
    PeerReg(usize, peer_con::WeakPeerCon),
    PeerUnreg(usize),
    DataReg(usize, data_chan::WeakDataChan),
    DataUnreg(usize),
}

struct Manager {
    rt_send: std::sync::mpsc::Sender<Cmd>,
}

impl Manager {
    pub fn new() -> Self {
        struct D;

        impl Drop for D {
            fn drop(&mut self) {
                tracing::error!("tx5-go-pion offload EVENT LOOP EXITED");
            }
        }

        let drop_trace = D;

        let (rt_send, rt_recv) = std::sync::mpsc::channel();

        // we need to offload the event handling to another thread
        // because it's not safe to invoke other go apis within the
        // same sync call:
        // https://github.com/pion/webrtc/issues/2404
        std::thread::Builder::new()
            .name("tx5-evt-offload".to_string())
            .spawn(move || {
                let _drop_trace = drop_trace;

                let mut peer_map: HashMap<usize, peer_con::WeakPeerCon> =
                    HashMap::new();
                let mut data_map: HashMap<usize, data_chan::WeakDataChan> =
                    HashMap::new();

                trait Wk<E> {
                    fn se(&self, evt: E) -> Result<()>;
                }

                impl Wk<PeerConnectionEvent> for peer_con::WeakPeerCon {
                    fn se(&self, evt: PeerConnectionEvent) -> Result<()> {
                        self.send_evt(evt)
                    }
                }

                impl Wk<DataChannelEvent> for data_chan::WeakDataChan {
                    fn se(&self, evt: DataChannelEvent) -> Result<()> {
                        self.send_evt(evt)
                    }
                }

                fn smap<E, W: Wk<E>>(
                    map: &mut HashMap<usize, W>,
                    id: usize,
                    evt: E,
                ) {
                    if let std::collections::hash_map::Entry::Occupied(o) =
                        map.entry(id)
                    {
                        if o.get().se(evt).is_err() {
                            o.remove();
                        }
                    }
                }

                while let Ok(cmd) = rt_recv.recv() {
                    match cmd {
                        Cmd::Evt(evt) => match evt {
                            SysEvent::Error(error) => {
                                tracing::error!(
                                    ?error,
                                    "tx5-go-pion error event"
                                );
                            }
                            SysEvent::PeerConICECandidate {
                                peer_con_id,
                                candidate,
                            } => {
                                smap(
                                    &mut peer_map,
                                    peer_con_id,
                                    PeerConnectionEvent::ICECandidate(GoBuf(
                                        candidate,
                                    )),
                                );
                            }
                            SysEvent::PeerConStateChange {
                                peer_con_id,
                                peer_con_state,
                            } => {
                                smap(
                                    &mut peer_map,
                                    peer_con_id,
                                    PeerConnectionEvent::State(
                                        PeerConnectionState::from_raw(
                                            peer_con_state,
                                        ),
                                    ),
                                );
                            }
                            SysEvent::PeerConDataChan {
                                peer_con_id,
                                data_chan_id,
                            } => {
                                let (chan, recv) =
                                    DataChannel::new(data_chan_id);
                                smap(
                                    &mut peer_map,
                                    peer_con_id,
                                    PeerConnectionEvent::DataChannel(
                                        chan, recv,
                                    ),
                                );
                            }
                            SysEvent::DataChanClose(data_chan_id) => {
                                smap(
                                    &mut data_map,
                                    data_chan_id,
                                    DataChannelEvent::Close,
                                );
                            }
                            SysEvent::DataChanOpen(data_chan_id) => {
                                smap(
                                    &mut data_map,
                                    data_chan_id,
                                    DataChannelEvent::Open,
                                );
                            }
                            SysEvent::DataChanMessage {
                                data_chan_id,
                                buffer_id,
                            } => {
                                let buf = GoBuf(buffer_id);
                                smap(
                                    &mut data_map,
                                    data_chan_id,
                                    DataChannelEvent::Message(buf),
                                );
                            }
                            SysEvent::DataChanBufferedAmountLow(
                                data_chan_id,
                            ) => {
                                smap(
                                    &mut data_map,
                                    data_chan_id,
                                    DataChannelEvent::BufferedAmountLow,
                                );
                            }
                        },
                        Cmd::PeerReg(id, peer_con) => {
                            peer_map.insert(id, peer_con);
                        }
                        Cmd::PeerUnreg(id) => {
                            peer_map.remove(&id);
                        }
                        Cmd::DataReg(id, data_chan) => {
                            data_map.insert(id, data_chan);
                        }
                        Cmd::DataUnreg(id) => {
                            data_map.remove(&id);
                        }
                    }
                }
            })
            .expect("Failed to spawn offload thread");

        Self { rt_send }
    }

    pub fn register_peer_con(
        &self,
        id: usize,
        peer_con: peer_con::WeakPeerCon,
    ) {
        let _ = self.rt_send.send(Cmd::PeerReg(id, peer_con));
    }

    pub fn unregister_peer_con(&self, id: usize) {
        let _ = self.rt_send.send(Cmd::PeerUnreg(id));
    }

    pub fn register_data_chan(
        &self,
        id: usize,
        data_chan: data_chan::WeakDataChan,
    ) {
        let _ = self.rt_send.send(Cmd::DataReg(id, data_chan));
    }

    pub fn unregister_data_chan(&self, id: usize) {
        let _ = self.rt_send.send(Cmd::DataUnreg(id));
    }

    pub fn handle_event(&self, evt: SysEvent) {
        let _ = self.rt_send.send(Cmd::Evt(evt));
    }
}
pub(crate) fn register_peer_con(id: usize, peer_con: peer_con::WeakPeerCon) {
    MANAGER.register_peer_con(id, peer_con);
}

pub(crate) fn unregister_peer_con(id: usize) {
    MANAGER.unregister_peer_con(id);
}

pub(crate) fn register_data_chan(
    id: usize,
    data_chan: data_chan::WeakDataChan,
) {
    MANAGER.register_data_chan(id, data_chan);
}

pub(crate) fn unregister_data_chan(id: usize) {
    MANAGER.unregister_data_chan(id);
}

static MANAGER: Lazy<Manager> = Lazy::new(|| {
    unsafe {
        API.on_event(move |evt| {
            MANAGER.handle_event(evt);
        });
    }

    Manager::new()
});
