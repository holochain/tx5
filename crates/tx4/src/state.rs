//! Tx4 high-level conn mgmt state.

use crate::*;

use std::sync::Arc;
use tx4_core::Id;

mod store;
pub(crate) use store::*;

mod state_data;
pub(crate) use state_data::*;

mod sig_state;
pub use sig_state::*;

/// Respond type.
pub struct OneSnd<T: 'static + Send>(
    Option<Box<dyn FnOnce(T) + 'static + Send>>,
);

impl<T: 'static + Send> OneSnd<T> {
    #[allow(dead_code)]
    pub(crate) fn new<Cb>(cb: Cb) -> Self
    where
        Cb: FnOnce(T) + 'static + Send,
    {
        Self(Some(Box::new(cb)))
    }

    /// Send data on this single sender respond type.
    pub fn send(&mut self, t: T) {
        if let Some(sender) = self.0.take() {
            sender(t);
        }
    }

    /// Wrap such that a closure's result is sent.
    /// This is especially useful when `T` is a `Result<_>`.
    pub fn with<Cb>(&mut self, cb: Cb)
    where
        Cb: FnOnce() -> T,
    {
        self.send(cb());
    }
}

/// Generic receiver type.
pub struct ManyRcv<T: 'static + Send>(tokio::sync::mpsc::UnboundedReceiver<T>);

impl<T: 'static + Send> ManyRcv<T> {
    /// Receive data from this receiver type.
    #[inline]
    pub async fn recv(&mut self) -> Option<T> {
        tokio::sync::mpsc::UnboundedReceiver::recv(&mut self.0).await
    }
}

/// State wishes to invoke an action.
pub enum StateEvt {
    /// Request to create a new signal client connection.
    NewSig(SigStateSeed),

    /// Request to create a new webrtc peer connection.
    NewConn(ManyRcv<Result</*ConnStateEvt*/ bool>>),

    /// Incoming data received on a peer connection.
    /// The recv buffer will only decrement once the future resolves.
    RcvData(Arc<str>, Buf, OneSnd<()>),
}

/// Handle to a state tracking instance.
#[derive(Clone)]
pub struct State(StateData);

impl State {
    /// Construct a new state instance.
    pub fn new() -> (Self, ManyRcv<StateEvt>) {
        let (state_snd, state_rcv) = tokio::sync::mpsc::unbounded_channel();
        let state_data = StateData::new(state_snd);
        (Self(state_data), ManyRcv(state_rcv))
    }

    /// Shutdown the state system instance.
    pub fn shutdown(&self, _maybe_err: Option<Error>) {}

    /// Establish a new listening connection through signal server.
    pub async fn listener_sig(&self, url: Arc<str>) -> Result<()> {
        let (s, r) = tokio::sync::oneshot::channel();
        self.0.check_new_listener_sig(url, s)?;
        r.await.map_err(|_| Error::id("Closed"))?
    }

    /// Schedule data to be sent out over a channel managed by the state system.
    /// The future will resolve immediately if there is still space
    /// in the outgoing buffer, or once there is again space in the buffer.
    pub async fn snd_data(&self, _url: Arc<str>, _data: Buf) -> Result<()> {
        todo!()
    }
}

/*
use tx4_core::Id;
use parking_lot::Mutex;


mod sig_state;
pub use sig_state::*;

mod state;
pub use state::*;

/// Boxed future type.
pub type BoxFut<'lt, T> = std::pin::Pin<
    Box<dyn std::future::Future<Output = T> + 'lt + Send>,
>;

/// Indication of the current buffer state.
pub enum BufState {
    /// Buffer is low, we can buffer more data.
    Low,

    /// Buffer is high, we should wait / apply backpressure.
    High,
}


/// State wishes to invoke an action on a connection instance.
pub enum ConnStateEvt {
    /// Request to create an offer.
    CreateOffer(OneSnd<Result<Buf>>),

    /// Request to create an answer.
    CreateAnswer(OneSnd<Result<Buf>>),

    /// Request to set a local description.
    SetLoc(Buf, OneSnd<Result<()>>),

    /// Request to set a remote description.
    SetRem(Buf, OneSnd<Result<()>>),

    /// Request to append a trickle ICE candidate.
    SetIce(Buf, OneSnd<Result<()>>),

    /// Request to send a message on the data channel.
    SndData(Buf, OneSnd<Result<BufState>>),
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

*/
