//! Tx4 high-level conn mgmt state.

use crate::*;
use std::sync::Arc;
use tokio::sync::Mutex as TokioMutex;
use tx4_core::Id;

/// Boxed future type.
pub type BoxFut<'lt, T> = std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<T>> + 'lt + Send>,
>;

/// Indication of the current buffer state.
pub enum BufState {
    /// Buffer is low, we can buffer more data.
    Low,

    /// Buffer is high, we should wait / apply backpressure.
    High,
}

/// State wishes to invoke an action on a connection instance.
pub trait ConnStateEvt: 'static + Send {
    /// Request to create an offer.
    fn on_create_offer(&mut self) -> BoxFut<'_, Buf>;

    /// Request to create an answer.
    fn on_create_answer(&mut self) -> BoxFut<'_, Buf>;

    /// Request to set a local description.
    fn on_set_loc(&mut self, data: Buf) -> BoxFut<'_, ()>;

    /// Request to set a remote description.
    fn on_set_rem(&mut self, data: Buf) -> BoxFut<'_, ()>;

    /// Request to append a trickle ICE candidate.
    fn on_set_ice(&mut self, data: Buf) -> BoxFut<'_, ()>;

    /// Request to send a message on the data channel.
    fn on_snd_data(&mut self, data: Buf) -> BoxFut<'_, BufState>;
}

/// A handle for notifying the state system of connection events.
pub struct ConnState {}

impl ConnState {
    /// Shutdown the connection with an optional error.
    pub fn shutdown(&self, _maybe_err: Option<Error>) {
        todo!()
    }

    /// The connection generated an ice candidate for the remote.
    pub fn ice(&self, _data: Buf) -> Result<()> {
        todo!()
    }

    /// The connection received data on the data channel.
    /// This synchronous function must not block for now...
    /// (we'll need to test some blocking strategies
    /// for the goroutine in tx4-go-pion)... but we also can't just
    /// fill up memory if the application is processing slowly.
    /// So it will error / trigger connection shutdown if we get
    /// too much of a backlog.
    pub fn rcv_data(&self, _data: Buf) -> Result<()> {
        todo!()
    }

    /// The send buffer *was* high, but has now transitioned to low.
    pub fn buf_amt_low(&self) -> Result<()> {
        todo!()
    }
}

/// State wishes to invoke an action on a signal instance.
pub trait SigStateEvt: 'static + Send {
    /// Forward an offer to a remote.
    fn on_snd_offer(&mut self, id: Id, data: Buf) -> BoxFut<'_, ()>;

    /// Forward an answer to a remote.
    fn on_snd_answer(&mut self, id: Id, data: Buf) -> BoxFut<'_, ()>;

    /// Forward an ICE candidate to a remote
    fn on_snd_ice(&mut self, id: Id, data: Buf) -> BoxFut<'_, ()>;
}

/// A handle for notifying the state system of signal events.
pub struct SigState {}

impl SigState {
    fn new() -> Self {
        Self {}
    }

    /// Shutdown the signal client with an optional error.
    pub fn shutdown(&self, _maybe_err: Option<Error>) {
        todo!()
    }

    /// Receive an incoming offer from a remote.
    pub fn offer(&self, _id: Id, _data: Buf) -> Result<()> {
        todo!()
    }

    /// Receive an incoming answer from a remote.
    pub fn answer(&self, _id: Id, _data: Buf) -> Result<()> {
        todo!()
    }

    /// Receive an incoming ice candidate from a remote.
    pub fn ice(&self, _id: Id, _data: Buf) -> Result<()> {
        todo!()
    }
}

/// State wishes to invoke an action.
pub trait StateEvt<S, C>: 'static + Send
where
    S: SigStateEvt,
    C: ConnStateEvt,
{
    /// Request to create a new signal client connection.
    fn on_new_sig(
        &mut self,
        url: url::Url,
        sig_state: SigState,
    ) -> BoxFut<'_, S>;

    /// Request to create a new webrtc peer connection.
    fn on_new_conn(
        &mut self,
        url: url::Url,
        conn_state: ConnState,
    ) -> BoxFut<'_, C>;

    /// Incoming data received on a peer connection.
    /// The recv buffer will only decrement once the future resolves.
    fn on_rcv_data(&mut self, url: url::Url, data: Buf) -> BoxFut<'_, ()>;
}

/// Handle to a state tracking instance.
pub struct State<S, C, E>
where
    S: SigStateEvt,
    C: ConnStateEvt,
    E: StateEvt<S, C>,
{
    snd: Snd<StateCmd<S, C, E>>,
}

impl<S, C, E> State<S, C, E>
where
    S: SigStateEvt,
    C: ConnStateEvt,
    E: StateEvt<S, C>,
{
    /// Construct a new state instance.
    pub fn new(evt: E) -> Self {
        let (snd, rcv) = tokio::sync::mpsc::unbounded_channel();
        tokio::task::spawn(state_task(snd.clone(), evt, rcv));
        Self { snd }
    }

    /// Shutdown the state system instance with optional error.
    pub fn shutdown(&self, maybe_err: Option<Error>) {
        let _ = self.snd.send(StateCmd::Shutdown(maybe_err));
    }

    /// Establish a new listening connection through signal server.
    pub async fn listener_sig(&self, url: url::Url) -> Result<()> {
        let (snd, rcv) = tokio::sync::oneshot::channel();
        let _ = self.snd.send(StateCmd::ListenerSig(url, snd));
        rcv.await.map_err(|_| Error::id("Shutdown"))?
    }

    /// Schedule data to be sent out over a channel managed by the state system.
    /// The future will resolve immediately if there is still space
    /// in the outgoing buffer, or once there is again space in the buffer.
    pub async fn snd_data(&self, _url: url::Url, _data: Buf) -> Result<()> {
        todo!()
    }
}

type Snd<T> = tokio::sync::mpsc::UnboundedSender<T>;
type Rcv<T> = tokio::sync::mpsc::UnboundedReceiver<T>;
type SndOnce<T> = tokio::sync::oneshot::Sender<T>;

enum StateCmd<S, C, E>
where
    S: SigStateEvt,
    C: ConnStateEvt,
    E: StateEvt<S, C>,
{
    Shutdown(Option<Error>),
    ListenerSig(url::Url, SndOnce<Result<()>>),
    ListenerSigFinal(url::Url, S, SndOnce<Result<()>>),

    #[allow(dead_code)] // oy, c'mon rust...
    IgnoreMe(S, C, E),
}

async fn state_task<S, C, E>(
    snd: Snd<StateCmd<S, C, E>>,
    evt: E,
    mut rcv: Rcv<StateCmd<S, C, E>>,
) where
    S: SigStateEvt,
    C: ConnStateEvt,
    E: StateEvt<S, C>,
{
    let mut inner = StateInner::new(snd, evt);

    while let Some(cmd) = rcv.recv().await {
        if inner.cmd(cmd) {
            break;
        }
    }
}

struct StateInner<S, C, E>
where
    S: SigStateEvt,
    C: ConnStateEvt,
    E: StateEvt<S, C>,
{
    snd: Snd<StateCmd<S, C, E>>,
    evt: Arc<TokioMutex<E>>,
    _p: std::marker::PhantomData<(fn() -> S, fn() -> C)>,
}

impl<S, C, E> StateInner<S, C, E>
where
    S: SigStateEvt,
    C: ConnStateEvt,
    E: StateEvt<S, C>,
{
    fn new(snd: Snd<StateCmd<S, C, E>>, evt: E) -> Self {
        Self {
            snd,
            evt: Arc::new(TokioMutex::new(evt)),
            _p: std::marker::PhantomData,
        }
    }

    fn cmd(&mut self, cmd: StateCmd<S, C, E>) -> bool {
        match cmd {
            StateCmd::Shutdown(_) => return true,
            StateCmd::ListenerSig(url, resp) => {
                self.listener_sig(url, resp);
            }
            StateCmd::ListenerSigFinal(url, sig_evt, resp) => {
                let _ = resp.send(self.listener_sig_final(url, sig_evt));
            }
            StateCmd::IgnoreMe(_, _, _) => (),
        }
        false
    }

    fn listener_sig(&mut self, url: url::Url, resp: SndOnce<Result<()>>) {
        let snd = self.snd.clone();
        let evt = self.evt.clone();
        tokio::task::spawn(async move {
            let sig_state = SigState::new();
            let sig_evt = {
                let mut lock = evt.lock().await;
                match lock.on_new_sig(url.clone(), sig_state).await {
                    Err(err) => {
                        let _ = resp.send(Err(err));
                        return;
                    }
                    Ok(sig_evt) => sig_evt,
                }
            };
            let _ = snd.send(StateCmd::ListenerSigFinal(url, sig_evt, resp));
        });
    }

    fn listener_sig_final(
        &mut self,
        _url: url::Url,
        _sig_evt: S,
    ) -> Result<()> {
        todo!()
    }
}
