use super::*;

/// State wishes to invoke an action.
pub enum StateEvt {
    /// Request to create a new signal client connection.
    NewSig(ManyRcv<Result<SigStateEvt>>),

    /// Request to create a new webrtc peer connection.
    NewConn(ManyRcv<Result<ConnStateEvt>>),

    /// Incoming data received on a peer connection.
    /// The recv buffer will only decrement once the future resolves.
    RcvData(Arc<str>, Buf, OneSnd<()>),
}

/// Handle to a state tracking instance.
#[derive(Clone)]
pub struct State(Arc<Mutex<StateInner>>);

impl State {
    /// Construct a new state instance.
    pub fn new() -> (Self, ManyRcv<StateEvt>) {
        let (_state_snd, state_rcv) = tokio::sync::mpsc::unbounded_channel();
        let out = Arc::new(Mutex::new(StateInner {
            signal_map: HashMap::new(),
        }));
        (Self(out), ManyRcv(state_rcv))
    }

    /// Shutdown the state system instance.
    pub fn shutdown(&self, _maybe_err: Option<Error>) {
    }

    /// Establish a new listening connection through signal server.
    pub async fn listener_sig(&self, url: Arc<str>) -> Result<()> {
        self.0.lock().listener_sig(url).await.map_err(|_| Error::id("UnexpectedError"))?
    }

    /// Schedule data to be sent out over a channel managed by the state system.
    /// The future will resolve immediately if there is still space
    /// in the outgoing buffer, or once there is again space in the buffer.
    pub async fn snd_data(&self, _url: Arc<str>, _data: Buf) -> Result<()> {
        todo!()
    }
}

pub(crate) struct StateInner {
    signal_map: HashMap<Arc<str>, Arc<Mutex<SigStateInner>>>,
}

impl StateInner {
    pub fn listener_sig(&mut self, url: Arc<str>) -> tokio::sync::oneshot::Receiver<Result<()>> {
        let (s, r) = tokio::sync::oneshot::channel();
        match signal_map.entry(url) {
            hash_map::Entry::Occupied(e) => {
                // clone/spawn so we're not locked when we lock in turn.
                let ss_inner 
                let _ = s.send(Ok(()));
            }
            hash_map::Entry::Vacant(e) => {
                let sig_state_inner = SigStateInner {
                    init_ok: false,
                    init_cb_list: vec![s],
                };
                r
            }
        }
    }
}
