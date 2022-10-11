//! Tx4 high-level conn mgmt state.

use crate::*;

use std::collections::{hash_map, HashMap};
use std::future::Future;

use tx4_core::{Id, Tx4Url};

//mod store;
//pub(crate) use store::*;

//mod data;
//pub(crate) use data::*;

mod sig;
pub use sig::*;

mod conn;
pub use conn::*;

#[cfg(test)]
mod test;

/// Respond type.
#[must_use]
pub struct OneSnd<T: 'static + Send>(
    Option<Box<dyn FnOnce(Result<T>) + 'static + Send>>,
);

impl<T: 'static + Send> Drop for OneSnd<T> {
    fn drop(&mut self) {
        self.send(Err(Error::id("Dropped")))
    }
}

impl<T: 'static + Send> OneSnd<T> {
    #[allow(dead_code)]
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
    pub async fn with<Fut, Cb>(&mut self, cb: Cb)
    where
        Fut: Future<Output = Result<T>>,
        Cb: FnOnce() -> Fut,
    {
        self.send(cb().await);
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

#[derive(Clone)]
struct StateEvtSnd(tokio::sync::mpsc::UnboundedSender<Result<StateEvt>>);

impl StateEvtSnd {
    pub fn err(&self, err: std::io::Error) {
        let _ = self.0.send(Err(err));
    }

    pub fn publish(&self, evt: StateEvt) -> Result<()> {
        self.0.send(Ok(evt)).map_err(|_| Error::id("Closed"))
    }
}

struct StateData {
    #[allow(dead_code)]
    this: StateWeak,
    evt: StateEvtSnd,
    signal_map: HashMap<Tx4Url, SigState>,
    conn_map: HashMap<Id, ConnState>,
}

impl Drop for StateData {
    fn drop(&mut self) {
        self.shutdown(Error::id("Dropped"));
    }
}

impl StateData {
    fn shutdown(&mut self, err: std::io::Error) {
        for (_, sig) in self.signal_map.drain() {
            sig.close(err.err_clone());
        }
        for (_, conn) in self.conn_map.drain() {
            conn.close(err.err_clone());
        }
        self.evt.err(err);
    }

    async fn assert_listener_sig(
        &mut self,
        state: StateWeak,
        sig_url: Tx4Url,
        maybe_resp: Option<tokio::sync::oneshot::Sender<Result<()>>>,
    ) -> Result<SigState> {
        match self.signal_map.entry(sig_url.clone()) {
            hash_map::Entry::Occupied(e) => {
                let sig = e.get().clone();
                if let Some(resp) = maybe_resp {
                    sig.push_assert_respond(resp).await;
                }
                Ok(sig)
            }
            hash_map::Entry::Vacant(e) => {
                let (sig, sig_evt) =
                    SigState::new(state, sig_url.clone(), maybe_resp);
                e.insert(sig.clone());
                let seed = SigStateSeed::new(sig.clone(), sig_evt);
                let _ = self.evt.publish(StateEvt::NewSig(seed));
                Ok(sig)
            }
        }
    }

    async fn assert_conn(
        &mut self,
        state: StateWeak,
        cli_url: Tx4Url,
        maybe_send: Option<(Buf, tokio::sync::oneshot::Sender<Result<()>>)>,
    ) -> Result<()> {
        let have_send = maybe_send.is_some();
        let rem_id = cli_url.id().unwrap();

        if let Some((_data, _snd_resp)) = maybe_send {
            todo!()
        }

        if let Some(e) = self.conn_map.get(&rem_id) {
            if have_send {
                e.notify_send_waiting().await;
            }
            return Ok(());
        }

        let sig_url = cli_url.to_server();
        let (s, r) = tokio::sync::oneshot::channel();
        let sig = self
            .assert_listener_sig(state.clone(), sig_url, Some(s))
            .await?;

        let (conn, conn_evt) = ConnState::new(state, sig.weak(), rem_id, r);
        self.conn_map.insert(rem_id, conn.clone());
        let seed = ConnStateSeed::new(conn, conn_evt);
        let _ = self.evt.publish(StateEvt::NewConn(seed));
        Ok(())
    }
}

/// Weak version of State.
#[derive(Clone, PartialEq, Eq)]
pub struct StateWeak(ActorWeak<StateData>);

impl PartialEq<State> for StateWeak {
    fn eq(&self, rhs: &State) -> bool {
        self.0 == rhs.0
    }
}

impl StateWeak {
    /// Upgrade to a full State instance.
    pub fn upgrade(&self) -> Option<State> {
        self.0.upgrade().map(State)
    }
}

/// Handle to a state tracking instance.
#[derive(Clone)]
pub struct State(Actor<StateData>);

impl PartialEq<StateWeak> for State {
    fn eq(&self, rhs: &StateWeak) -> bool {
        self.0 == rhs.0
    }
}

impl State {
    /// Construct a new state instance.
    pub fn new() -> (Self, ManyRcv<Result<StateEvt>>) {
        let (state_snd, state_rcv) = tokio::sync::mpsc::unbounded_channel();
        let actor = Actor::new(|this| StateData {
            this: StateWeak(this),
            evt: StateEvtSnd(state_snd),
            signal_map: HashMap::new(),
            conn_map: HashMap::new(),
        });
        (Self(actor), ManyRcv(state_rcv))
    }

    /// Get a weak version of this State instance.
    pub fn weak(&self) -> StateWeak {
        StateWeak(self.0.weak())
    }

    /// Returns `true` if this SigState is closed.
    pub fn is_closed(&self) -> bool {
        self.0.is_closed()
    }

    /// Shutdown the signal client with an error.
    pub fn close(&self, err: std::io::Error) {
        tokio::task::spawn(self.0.exec_close(|mut inner| async move {
            inner.shutdown(err);
            (None, Ok(()))
        }));
    }

    /// Establish a new listening connection through given signal server.
    pub fn listener_sig(
        &self,
        sig_url: Tx4Url,
    ) -> impl Future<Output = Result<()>> + 'static + Send {
        let this = self.clone();
        async move {
            if !sig_url.is_server() {
                return Err(Error::err(
                    "Invalid tx4 client url, expected signal server url",
                ));
            }
            let (s, r) = tokio::sync::oneshot::channel();
            let state = this.weak();
            this.0
                .exec(move |mut inner| async move {
                    let r = inner
                        .assert_listener_sig(state, sig_url, Some(s))
                        .await;
                    (Some(inner), r)
                })
                .await?;
            r.await.map_err(|_| Error::id("Closed"))?
        }
    }

    /// Schedule data to be sent out over a channel managed by the state system.
    /// The future will resolve immediately if there is still space
    /// in the outgoing buffer, or once there is again space in the buffer.
    pub fn snd_data(
        &self,
        cli_url: Tx4Url,
        data: Buf,
    ) -> impl Future<Output = Result<()>> + 'static + Send {
        let this = self.clone();
        async move {
            if !cli_url.is_client() {
                return Err(Error::err(
                    "Invalid tx4 signal server url, expect client url",
                ));
            }

            let (s, r) = tokio::sync::oneshot::channel();

            let state = this.weak();
            this.0
                .exec(move |mut inner| async move {
                    let r = inner
                        .assert_conn(state, cli_url, Some((data, s)))
                        .await;
                    (Some(inner), r)
                })
                .await?;

            r.await.map_err(|_| Error::id("Closed"))?
        }
    }

    // -- //

    pub(crate) async fn publish(&self, evt: StateEvt) -> Result<()> {
        self.0
            .exec(move |inner| async move {
                let r = inner.evt.publish(evt);
                (Some(inner), r)
            })
            .await
    }

    pub(crate) fn close_sig(
        &self,
        _sig_url: Tx4Url,
        _sig: SigStateWeak,
        _err: std::io::Error,
    ) {
        todo!()
    }

    pub(crate) fn close_conn(
        &self,
        _conn: ConnStateWeak,
        _err: std::io::Error,
    ) {
        todo!()
    }
}

/*
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

    /// Establish a new listening connection through given signal server.
    pub async fn listener_sig(&self, url: Tx4Url) -> Result<()> {
        if !url.is_server() {
            return Err(Error::err(
                "Invalid tx4 client url, expected signal server url",
            ));
        }
        self.0.assert_listener_sig(url).await
    }

    /// Schedule data to be sent out over a channel managed by the state system.
    /// The future will resolve immediately if there is still space
    /// in the outgoing buffer, or once there is again space in the buffer.
    pub async fn snd_data(&self, url: Tx4Url, data: Buf) -> Result<()> {
        if !url.is_client() {
            return Err(Error::err(
                "Invalid tx4 signal server url, expect client url",
            ));
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
*/
