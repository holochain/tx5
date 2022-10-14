//! Tx4 high-level conn mgmt state.

use crate::actor::*;
use crate::*;

use std::collections::{hash_map, HashMap};
use std::future::Future;
use std::sync::Arc;

use tx4_core::{Id, Tx4Url};

mod sig;
pub use sig::*;

mod conn;
pub use conn::*;

#[cfg(test)]
mod test;

const SEND_LIMIT: u32 = 16 * 1024 * 1024;

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

struct SendData {
    #[allow(dead_code)]
    data: Buf,
    timestamp: std::time::Instant,
    resp: Option<tokio::sync::oneshot::Sender<Result<()>>>,
    _send_permit: tokio::sync::OwnedSemaphorePermit,
}

struct StateData {
    this: StateWeak,
    evt: StateEvtSnd,
    signal_map: HashMap<Tx4Url, SigStateWeak>,
    conn_map: HashMap<Id, ConnStateWeak>,
    send_map: HashMap<Id, Vec<SendData>>,
}

impl Drop for StateData {
    fn drop(&mut self) {
        self.shutdown(Error::id("Dropped"));
    }
}

impl StateData {
    fn shutdown(&mut self, err: std::io::Error) {
        for (_, sig) in self.signal_map.drain() {
            if let Some(sig) = sig.upgrade() {
                sig.close(err.err_clone());
            }
        }
        for (_, conn) in self.conn_map.drain() {
            if let Some(conn) = conn.upgrade() {
                conn.close(err.err_clone());
            }
        }
        self.evt.err(err);
    }

    async fn exec(&mut self, cmd: StateCmd) -> Result<()> {
        match cmd {
            StateCmd::Tick => self.tick().await,
            StateCmd::AssertListenerSig { sig_url, resp } => {
                self.assert_listener_sig(sig_url, resp).await
            }
            StateCmd::SendData {
                rem_id,
                data,
                send_permit,
                resp,
                cli_url,
            } => {
                self.send_data(rem_id, data, send_permit, resp, cli_url)
                    .await
            }
            StateCmd::Publish { evt } => self.publish(evt).await,
            StateCmd::CloseSig { sig_url, sig, err } => {
                self.close_sig(sig_url, sig, err).await
            }
            StateCmd::CloseConn { rem_id, conn, err } => {
                self.close_conn(rem_id, conn, err).await
            }
        }
    }

    async fn tick(&mut self) -> Result<()> {
        self.send_map.retain(|_, list| {
            list.retain_mut(|info| {
                if info.timestamp.elapsed() < std::time::Duration::from_secs(20)
                {
                    true
                } else {
                    if let Some(resp) = info.resp.take() {
                        let _ = resp.send(Err(Error::id("Timeout")));
                    }
                    false
                }
            });
            !list.is_empty()
        });
        Ok(())
    }

    async fn assert_listener_sig(
        &mut self,
        sig_url: Tx4Url,
        resp: tokio::sync::oneshot::Sender<Result<()>>,
    ) -> Result<()> {
        let new_sig = |resp| -> SigState {
            let (sig, sig_evt) =
                SigState::new(self.this.clone(), sig_url.clone(), resp);
            let seed = SigStateSeed::new(sig.clone(), sig_evt);
            let _ = self.evt.publish(StateEvt::NewSig(seed));
            sig
        };
        match self.signal_map.entry(sig_url.clone()) {
            hash_map::Entry::Occupied(mut e) => match e.get().upgrade() {
                Some(sig) => sig.push_assert_respond(resp).await,
                None => {
                    let sig = new_sig(resp);
                    e.insert(sig.weak());
                }
            },
            hash_map::Entry::Vacant(e) => {
                let sig = new_sig(resp);
                e.insert(sig.weak());
            }
        }
        Ok(())
    }

    async fn send_data(
        &mut self,
        rem_id: Id,
        data: Buf,
        send_permit: tokio::sync::OwnedSemaphorePermit,
        data_sent: tokio::sync::oneshot::Sender<Result<()>>,
        cli_url: Tx4Url,
    ) -> Result<()> {
        self.send_map.entry(rem_id).or_default().push(SendData {
            data,
            timestamp: std::time::Instant::now(),
            resp: Some(data_sent),
            _send_permit: send_permit,
        });

        let rem_id = cli_url.id().unwrap();

        if let Some(e) = self.conn_map.get(&rem_id) {
            if let Some(conn) = e.upgrade() {
                conn.notify_send_waiting().await;
                return Ok(());
            } else {
                self.conn_map.remove(&rem_id);
            }
        }

        let sig_url = cli_url.to_server();
        let (s, r) = tokio::sync::oneshot::channel();
        self.assert_listener_sig(sig_url.clone(), s).await?;

        let sig = self.signal_map.get(&sig_url).unwrap().clone();

        let (conn, conn_evt) =
            ConnState::new(self.this.clone(), sig, rem_id, r);
        self.conn_map.insert(rem_id, conn.weak());
        let seed = ConnStateSeed::new(conn, conn_evt);
        let _ = self.evt.publish(StateEvt::NewConn(seed));
        Ok(())
    }

    async fn publish(&mut self, evt: StateEvt) -> Result<()> {
        let _ = self.evt.publish(evt);
        Ok(())
    }

    async fn close_sig(
        &mut self,
        sig_url: Tx4Url,
        sig: SigStateWeak,
        err: std::io::Error,
    ) -> Result<()> {
        if let Some(cur_sig) = self.signal_map.remove(&sig_url) {
            if cur_sig == sig {
                if let Some(sig) = sig.upgrade() {
                    sig.close(err);
                }
            } else {
                // Whoops!
                self.signal_map.insert(sig_url, sig);
            }
        }
        Ok(())
    }

    async fn close_conn(
        &mut self,
        rem_id: Id,
        conn: ConnStateWeak,
        err: std::io::Error,
    ) -> Result<()> {
        if let Some(cur_conn) = self.conn_map.remove(&rem_id) {
            if cur_conn == conn {
                if let Some(conn) = conn.upgrade() {
                    conn.close(err);
                }
            } else {
                // Whoops!
                self.conn_map.insert(rem_id, conn);
            }
        }
        Ok(())
    }
}

enum StateCmd {
    Tick,
    AssertListenerSig {
        sig_url: Tx4Url,
        resp: tokio::sync::oneshot::Sender<Result<()>>,
    },
    SendData {
        rem_id: Id,
        data: Buf,
        send_permit: tokio::sync::OwnedSemaphorePermit,
        resp: tokio::sync::oneshot::Sender<Result<()>>,
        cli_url: Tx4Url,
    },
    Publish {
        evt: StateEvt,
    },
    CloseSig {
        sig_url: Tx4Url,
        sig: SigStateWeak,
        err: std::io::Error,
    },
    CloseConn {
        rem_id: Id,
        conn: ConnStateWeak,
        err: std::io::Error,
    },
}

async fn state_task(
    mut rcv: ManyRcv<StateCmd>,
    this: StateWeak,
    evt: StateEvtSnd,
) -> Result<()> {
    let mut data = StateData {
        this,
        evt,
        signal_map: HashMap::new(),
        conn_map: HashMap::new(),
        send_map: HashMap::new(),
    };
    let err = match async {
        while let Some(cmd) = rcv.recv().await {
            data.exec(cmd?).await?;
        }
        Ok(())
    }
    .await
    {
        Err(err) => err,
        Ok(_) => Error::id("Dropped"),
    };
    data.shutdown(err.err_clone());
    Err(err)
}

/// Weak version of State.
#[derive(Clone)]
pub struct StateWeak(ActorWeak<StateCmd>, Arc<tokio::sync::Semaphore>);

impl StateWeak {
    /// Upgrade to a full State instance.
    pub fn upgrade(&self) -> Option<State> {
        self.0.upgrade().map(|s| State(s, self.1.clone()))
    }
}

/// Handle to a state tracking instance.
#[derive(Clone)]
pub struct State(Actor<StateCmd>, Arc<tokio::sync::Semaphore>);

impl State {
    /// Construct a new state instance.
    pub fn new() -> (Self, ManyRcv<StateEvt>) {
        let send_limit =
            Arc::new(tokio::sync::Semaphore::new(SEND_LIMIT as usize));

        let (state_snd, state_rcv) = tokio::sync::mpsc::unbounded_channel();
        let actor = {
            let send_limit = send_limit.clone();
            Actor::new(move |this, rcv| {
                state_task(
                    rcv,
                    StateWeak(this, send_limit),
                    StateEvtSnd(state_snd),
                )
            })
        };

        let weak = StateWeak(actor.weak(), send_limit.clone());
        tokio::task::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                match weak.upgrade() {
                    None => break,
                    Some(actor) => {
                        if actor.tick().is_err() {
                            break;
                        }
                    }
                }
            }
        });

        (Self(actor, send_limit), ManyRcv(state_rcv))
    }

    /// Get a weak version of this State instance.
    pub fn weak(&self) -> StateWeak {
        StateWeak(self.0.weak(), self.1.clone())
    }

    /// Returns `true` if this SigState is closed.
    pub fn is_closed(&self) -> bool {
        self.0.is_closed()
    }

    /// Shutdown the signal client with an error.
    pub fn close(&self, err: std::io::Error) {
        self.0.close(err);
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
            this.0
                .send(Ok(StateCmd::AssertListenerSig { sig_url, resp: s }))?;
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
        let mut data = data;
        let send_limit = self.1.clone();
        async move {
            if !cli_url.is_client() {
                return Err(Error::err(
                    "Invalid tx4 signal server url, expect client url",
                ));
            }

            let len = data.len()?;
            if len > SEND_LIMIT as usize {
                return Err(Error::id("DataTooLarge"));
            }
            let send_permit = send_limit
                .acquire_many_owned(len as u32)
                .await
                .map_err(Error::err)?;

            let rem_id = cli_url.id().unwrap();

            let (s_sent, r_sent) = tokio::sync::oneshot::channel();
            this.0.send(Ok(StateCmd::SendData {
                rem_id,
                data,
                send_permit,
                resp: s_sent,
                cli_url,
            }))?;

            r_sent.await.map_err(|_| Error::id("Closed"))?
        }
    }

    // -- //

    fn tick(&self) -> Result<()> {
        self.0.send(Ok(StateCmd::Tick))
    }

    pub(crate) fn publish(&self, evt: StateEvt) {
        let _ = self.0.send(Ok(StateCmd::Publish { evt }));
    }

    pub(crate) fn close_sig(
        &self,
        sig_url: Tx4Url,
        sig: SigStateWeak,
        err: std::io::Error,
    ) {
        let _ = self.0.send(Ok(StateCmd::CloseSig { sig_url, sig, err }));
    }

    pub(crate) fn close_conn(
        &self,
        rem_id: Id,
        conn: ConnStateWeak,
        err: std::io::Error,
    ) {
        let _ = self.0.send(Ok(StateCmd::CloseConn { rem_id, conn, err }));
    }
}
