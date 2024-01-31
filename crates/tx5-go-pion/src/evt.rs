use crate::*;
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
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

pub(crate) use tx5_core::EventSend;
pub use tx5_core::{EventPermit, EventRecv};

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
    DataChannel(DataChannel, EventRecv<DataChannelEvent>),
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
    Message(GoBuf, tokio::sync::OwnedSemaphorePermit),

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

struct Manager {
    peer_con: HashMap<usize, peer_con::WeakPeerCon>,
    data_chan: HashMap<usize, data_chan::WeakDataChan>,
}

impl Manager {
    pub fn new() -> Mutex<Self> {
        Mutex::new(Self {
            peer_con: HashMap::new(),
            data_chan: HashMap::new(),
        })
    }

    pub fn register_peer_con(
        &mut self,
        id: usize,
        peer_con: peer_con::WeakPeerCon,
    ) {
        self.peer_con.insert(id, peer_con);
    }

    pub fn unregister_peer_con(&mut self, id: usize) {
        self.peer_con.remove(&id);
    }

    pub fn register_data_chan(
        &mut self,
        id: usize,
        data_chan: data_chan::WeakDataChan,
    ) {
        self.data_chan.insert(id, data_chan);
    }

    pub fn unregister_data_chan(&mut self, id: usize) {
        self.data_chan.remove(&id);
    }
}
pub(crate) fn register_peer_con(id: usize, peer_con: peer_con::WeakPeerCon) {
    MANAGER.lock().unwrap().register_peer_con(id, peer_con);
}

pub(crate) fn unregister_peer_con(id: usize) {
    MANAGER.lock().unwrap().unregister_peer_con(id);
}

pub(crate) fn register_data_chan(
    id: usize,
    data_chan: data_chan::WeakDataChan,
) {
    MANAGER.lock().unwrap().register_data_chan(id, data_chan);
}

pub(crate) fn unregister_data_chan(id: usize) {
    MANAGER.lock().unwrap().unregister_data_chan(id);
}

struct EvtOffload {
    handle: tokio::runtime::Handle,
    limit: Arc<tokio::sync::Semaphore>,
    _drop_s: tokio::sync::oneshot::Sender<()>,
}

impl Default for EvtOffload {
    fn default() -> Self {
        struct D;

        impl Drop for D {
            fn drop(&mut self) {
                tracing::error!("tx5-go-pion offload EVENT LOOP EXITED");
            }
        }

        let drop_trace = D;

        let (hand_s, hand_r) = std::sync::mpsc::sync_channel(0);

        let (_drop_s, drop_r) = tokio::sync::oneshot::channel();

        // we need to offload the event handling to another thread
        // because it's not safe to invoke other go apis within the
        // same sync call:
        // https://github.com/pion/webrtc/issues/2404
        std::thread::Builder::new()
            .name("tx5-evt-offload".to_string())
            .spawn(move || {
                // we need a custom runtime so tasks within it survive
                // the closing of e.g. a #tokio::test runtime
                tokio::runtime::Builder::new_current_thread()
                    .enable_time()
                    .build()
                    .expect("Failed to build tokio runtime")
                    .block_on(async move {
                        let _drop_trace = drop_trace;

                        let _ = hand_s.send(tokio::runtime::Handle::current());
                        // make sure this runtime/thread are dropped
                        // if self (and sender) are dropped.
                        let _ = drop_r.await;
                    });
            })
            .expect("Failed to spawn offload thread");

        let handle = hand_r.recv().unwrap();

        Self {
            // we will spawn offload tasks using this handle
            handle,
            // We don't want this "channel" to represent a significant
            // memory buffer, but we also don't want it to be so small
            // that it causes thread thrashing. Try 128 to start??
            // max msg size is 16 KiB, so 16 * 128 = 2 MiB.
            limit: Arc::new(tokio::sync::Semaphore::new(128)),
            // capture just so the receive side will error when this is dropped
            _drop_s,
        }
    }
}

impl EvtOffload {
    pub fn blocking_send<F>(&self, f: F)
    where
        F: Future<Output = ()> + 'static + Send,
    {
        // work around the lack of blocking_aquire in semaphores
        // by using a oneshot channel, and doing the acquire *in* the task
        let (s, r) = tokio::sync::oneshot::channel();
        let limit = self.limit.clone();
        // spawn using the offload thread runtime handle
        self.handle.spawn(async move {
            // get the permit
            let _permit = limit.acquire().await;
            // notify the permit was acquired
            let _ = s.send(());
            // run the future on the offload thread
            f.await;
        });
        // we have to do this blocking outside the task
        // so that the go thread trying to send the event is blocked
        let _ = r.blocking_recv();
    }
}

/*
macro_rules! manager_access {
    ($id:ident, $rt:ident, $map:ident, $code:expr) => {
        let $map = MANAGER.lock().unwrap().$map.get(&$id).cloned();
        if let Some($map) = &$map {
            let start = std::time::Instant::now();
            let result = $code.await;
            let elapsed_s = start.elapsed().as_secs_f64();
            if elapsed_s > 0.018 {
                tracing::error!(%elapsed_s, ?result, "SlowEvent");
            }
            if let Err(err) = result {
                $map.close(err.into());
            }
            /*
            match tokio::time::timeout(
                std::time::Duration::from_millis(18),
                $code,
            )
            .await
            .map_err(|_| Error::id("AppSlow"))
            {
                Err(err) | Ok(Err(err)) => {
                    tracing::error!(?err, "PionEvent");
                    $map.close(err.into());
                }
                Ok(_) => (),
            }
            */
        }
    };
}
*/

macro_rules! manager_access {
    ($id:ident, $off:ident, $map:ident, $code:expr) => {
        let $map = MANAGER.lock().unwrap().$map.get(&$id).cloned();
        if let Some($map) = $map {
            $off.blocking_send(async move {
                let start = std::time::Instant::now();
                let result = $code.await;
                let elapsed_s = start.elapsed().as_secs_f64();
                if elapsed_s > 0.018 {
                    tracing::error!(%elapsed_s, ?result, "SlowEvent");
                }
                if let Err(err) = result {
                    $map.close(err.into());
                }
            });
        }
    };
}

static MANAGER: Lazy<Mutex<Manager>> = Lazy::new(|| {
    let offload = EvtOffload::default();

    unsafe {
        API.on_event(move |sys_evt| match sys_evt {
            SysEvent::Error(_error) => (),
            SysEvent::PeerConICECandidate {
                peer_con_id,
                candidate,
            } => {
                manager_access!(peer_con_id, offload, peer_con, {
                    peer_con.send_evt(PeerConnectionEvent::ICECandidate(GoBuf(
                        candidate,
                    )))
                });
            }
            SysEvent::PeerConStateChange {
                peer_con_id,
                peer_con_state,
            } => {
                manager_access!(peer_con_id, offload, peer_con, {
                    peer_con.send_evt(PeerConnectionEvent::State(
                        PeerConnectionState::from_raw(peer_con_state),
                    ))
                });
            }
            SysEvent::PeerConDataChan {
                peer_con_id,
                data_chan_id,
            } => {
                manager_access!(peer_con_id, offload, peer_con, async {
                    let recv_limit = match peer_con.get_recv_limit() {
                        Ok(recv_limit) => recv_limit,
                        Err(err) => {
                            API.data_chan_free(data_chan_id);
                            return Err(err);
                        }
                    };

                    let (chan, recv) =
                        DataChannel::new(data_chan_id, recv_limit);

                    peer_con
                        .send_evt(PeerConnectionEvent::DataChannel(chan, recv))
                        .await
                });
            }
            SysEvent::DataChanClose(data_chan_id) => {
                manager_access!(
                    data_chan_id,
                    offload,
                    data_chan,
                    data_chan.send_evt(DataChannelEvent::Close)
                );
            }
            SysEvent::DataChanOpen(data_chan_id) => {
                manager_access!(
                    data_chan_id,
                    offload,
                    data_chan,
                    data_chan.send_evt(DataChannelEvent::Open)
                );
            }
            SysEvent::DataChanMessage {
                data_chan_id,
                buffer_id,
            } => {
                let mut buf = GoBuf(buffer_id);
                manager_access!(data_chan_id, offload, data_chan, async {
                    let len = buf.len()?;
                    if len > 16 * 1024 {
                        return Err(Error::id("MsgTooLarge"));
                    }

                    let recv_limit = data_chan.get_recv_limit()?;

                    let permit = recv_limit
                        .acquire_many_owned(len as u32)
                        .await
                        .map_err(|_| {
                            Error::from(Error::id(
                                "DataChanMessageSemaphoreClosed",
                            ))
                        })?;

                    data_chan
                        .send_evt(DataChannelEvent::Message(buf, permit))
                        .await
                });
            }
            SysEvent::DataChanBufferedAmountLow(data_chan_id) => {
                manager_access!(
                    data_chan_id,
                    offload,
                    data_chan,
                    data_chan.send_evt(DataChannelEvent::BufferedAmountLow,)
                );
            }
        });
    }

    Manager::new()
});
