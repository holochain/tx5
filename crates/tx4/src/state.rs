//! Tx4 high-level conn mgmt state.

use crate::*;

use tx4_core::{Id, Tx4Url};

//mod store;
//pub(crate) use store::*;

mod data;
pub(crate) use data::*;

mod sig;
pub use sig::*;

mod conn;
pub use conn::*;

#[cfg(test)]
mod test;

/// Respond type.
pub struct OneSnd<T: 'static + Send>(
    Option<Box<dyn FnOnce(Result<T>) + 'static + Send>>,
);

impl<T: 'static + Send> Drop for OneSnd<T> {
    fn drop(&mut self) {
        self.send(Err(Error::id("Dropped")))
    }
}

impl<T: 'static + Send> OneSnd<T> {
    pub(crate) fn new<Cb>(cb: Cb) -> Self
    where
        Cb: FnOnce(Result<T>) + 'static + Send,
    {
        Self(Some(Box::new(cb)))
    }

    /// Send data on this single sender respond type.
    pub fn send(&mut self, t: Result<T>) {
        if let Some(sender) = self.0.take() {
            sender(t);
        }
    }

    /// Wrap such that a closure's result is sent.
    /// This is especially useful when `T` is a `Result<_>`.
    pub fn with<Cb>(&mut self, cb: Cb)
    where
        Cb: FnOnce() -> Result<T>,
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

    /// Indicates the current node is addressable at the given url.
    Address(Tx4Url),

    /// Request to create a new webrtc peer connection.
    NewConn(ConnStateSeed),

    /// Incoming data received on a peer connection.
    RcvData(Tx4Url, Buf),
}

/// Handle to a state tracking instance.
#[derive(Clone)]
pub struct State(StateData);

impl State {
    /// Construct a new state instance.
    pub fn new() -> (Self, ManyRcv<Result<StateEvt>>) {
        let (state_snd, state_rcv) = tokio::sync::mpsc::unbounded_channel();
        let state_data = StateData::new(state_snd);
        (Self(state_data), ManyRcv(state_rcv))
    }

    /// Shutdown the state system instance.
    pub fn shutdown(&self, err: std::io::Error) {
        self.0.shutdown(err);
    }

    /// Establish a new listening connection through signal server.
    pub async fn listener_sig(&self, url: Tx4Url) -> Result<()> {
        if !url.is_server() {
            return Err(Error::err("Invalid tx4 client url, expected signal server url"));
        }
        self.0.assert_listener_sig(url).await
    }

    /// Schedule data to be sent out over a channel managed by the state system.
    /// The future will resolve immediately if there is still space
    /// in the outgoing buffer, or once there is again space in the buffer.
    pub async fn snd_data(&self, url: Tx4Url, data: Buf) -> Result<()> {
        if !url.is_client() {
            return Err(Error::err("Invalid tx4 signal server url, expect client url"));
        }

        // Verify open channel to signal server.
        // TODO - we may not need to be addressable at this possible other
        //        signal server...
        // TODO - if we have an open peer con, do we really need to
        //        first ensure an open signal server channel?
        let sig_url = url.to_server();
        self.listener_sig(sig_url).await?;

        // Now assert a connection is open, forwarding it the data to send.
        self.0.assert_conn_for_send(url, data).await
    }
}
