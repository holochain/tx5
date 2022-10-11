use super::*;
//use std::future::Future;
use std::collections::HashMap;

/// Temporary indicating we want a new signal instance.
pub struct SigStateSeed {
    done: bool,
    #[allow(dead_code)]
    sig_url: Tx4Url,
    sig: SigStateWeak,
    #[allow(dead_code)]
    sig_evt: Option<ManyRcv<Result<SigStateEvt>>>,
}

impl Drop for SigStateSeed {
    fn drop(&mut self) {
        self.result_err_inner(Error::id("Dropped"));
    }
}

impl SigStateSeed {
    /// Finalize this sig_state seed by indicating a successful sig connection.
    pub async fn result_ok(
        mut self,
        cli_url: Tx4Url,
    ) -> Result<(SigState, ManyRcv<Result<SigStateEvt>>)> {
        self.done = true;
        if let Some(sig) = self.sig.upgrade() {
            sig.notify_connected(cli_url).await?;
            Ok((sig, self.sig_evt.take().unwrap()))
        } else {
            Err(Error::id("Closed"))
        }
    }

    /// Finalize this sig_state seed by indicating an error connecting.
    pub fn result_err(mut self, err: std::io::Error) {
        self.result_err_inner(err);
    }

    // -- //

    pub(crate) fn new(
        sig_url: Tx4Url,
        sig: SigStateWeak,
        sig_evt: ManyRcv<Result<SigStateEvt>>,
    ) -> Self {
        Self {
            done: false,
            sig_url,
            sig,
            sig_evt: Some(sig_evt),
        }
    }

    fn result_err_inner(&mut self, err: std::io::Error) {
        if !self.done {
            self.done = true;

            if let Some(sig) = self.sig.upgrade() {
                sig.close(err);
            }
        }
    }
}

/*
/// Temporary indicating we want a new signal instance.
pub struct SigStateSeed {
    pub(crate) done: bool,
    pub(crate) key: Key,
    pub(crate) sig_url: Tx4Url,
    pub(crate) state_data: StateData,
    pub(crate) sig_evt: Option<ManyRcv<Result<SigStateEvt>>>,
}

impl Drop for SigStateSeed {
    fn drop(&mut self) {
        if !self.done {
            self.result_err_inner(Error::id("Dropped"));
        }
    }
}

impl SigStateSeed {
    /// Finalize this sig_state seed by indicating a successful sig connection.
    pub fn result_ok(
        mut self,
        cli_url: Tx4Url,
    ) -> (SigState, ManyRcv<Result<SigStateEvt>>) {
        self.done = true;
        self.state_data.new_listener_sig_ok(
            self.key,
            self.sig_url.clone(),
            cli_url,
        );
        let sig_state = SigState {
            inner: self.state_data.clone(),
            key: self.key,
            sig_url: self.sig_url.clone(),
        };
        let sig_evt = self.sig_evt.take().unwrap();
        (sig_state, sig_evt)
    }

    /// Finalize this sig_state seed by indicating an error connecting.
    pub fn result_err(mut self, err: std::io::Error) {
        self.result_err_inner(err);
    }

    fn result_err_inner(&mut self, err: std::io::Error) {
        self.done = true;
        self.state_data.new_listener_sig_err(
            self.key,
            self.sig_url.clone(),
            err,
        );
    }
}
*/

/// State wishes to invoke an action on a signal instance.
pub enum SigStateEvt {
    /// Forward an offer to a remote.
    SndOffer(Id, Buf, OneSnd<()>),

    /// Forward an answer to a remote.
    SndAnswer(Id, Buf, OneSnd<()>),

    /// Forward an ICE candidate to a remote
    SndIce(Id, Buf, OneSnd<()>),
}

#[derive(Clone)]
struct SigStateEvtSnd(tokio::sync::mpsc::UnboundedSender<Result<SigStateEvt>>);

impl SigStateEvtSnd {
    pub fn err(&self, err: std::io::Error) {
        let _ = self.0.send(Err(err));
    }
}

struct SigStateData {
    this: SigStateWeak,
    state: StateWeak,
    sig_url: Tx4Url,
    sig_evt: SigStateEvtSnd,
    connected: bool,
    pending_sig_resp: Vec<tokio::sync::oneshot::Sender<Result<()>>>,
    registered_conn_map: HashMap<Id, ConnStateWeak>,
}

impl Drop for SigStateData {
    fn drop(&mut self) {
        self.shutdown(Error::id("Dropped"));
    }
}

impl SigStateData {
    fn shutdown(&mut self, err: std::io::Error) {
        if let Some(state) = self.state.upgrade() {
            state.close_sig(
                self.sig_url.clone(),
                self.this.clone(),
                err.err_clone(),
            );
        }
        for (_, conn) in self.registered_conn_map.drain() {
            if let Some(conn) = conn.upgrade() {
                conn.close(err.err_clone());
            }
            drop(conn);
        }
        self.sig_evt.err(err);
    }
}

/// Weak version of SigState.
#[derive(Clone, PartialEq, Eq)]
pub struct SigStateWeak(ActorWeak<SigStateData>);

impl PartialEq<SigState> for SigStateWeak {
    fn eq(&self, rhs: &SigState) -> bool {
        self.0 == rhs.0
    }
}

impl SigStateWeak {
    /// Upgrade to a full SigState instance.
    pub fn upgrade(&self) -> Option<SigState> {
        self.0.upgrade().map(SigState)
    }
}

/// A handle for notifying the state system of signal events.
#[derive(Clone, PartialEq, Eq)]
pub struct SigState(Actor<SigStateData>);

impl PartialEq<SigStateWeak> for SigState {
    fn eq(&self, rhs: &SigStateWeak) -> bool {
        self.0 == rhs.0
    }
}

impl SigState {
    /// Get a weak version of this SigState instance.
    pub fn weak(&self) -> SigStateWeak {
        SigStateWeak(self.0.weak())
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

    /// Receive an incoming offer from a remote.
    pub fn offer(&self, rem_id: Id, data: Buf) -> Result<()> {
        // not super atomic, but a best effort : )
        if self.0.is_closed() {
            return Err(Error::id("Closed"));
        }
        tokio::task::spawn(self.0.exec(move |inner| async move {
            #[allow(clippy::redundant_pattern_matching)]
            if let Some(_) = inner.registered_conn_map.get(&rem_id) {
                // TODO : conn.offer(rem_id, data);
                drop(data);
            }
            (Some(inner), Ok(()))
        }));
        Ok(())
    }

    /// Receive an incoming answer from a remote.
    pub fn answer(&self, rem_id: Id, data: Buf) -> Result<()> {
        // not super atomic, but a best effort : )
        if self.0.is_closed() {
            return Err(Error::id("Closed"));
        }
        tokio::task::spawn(self.0.exec(move |inner| async move {
            #[allow(clippy::redundant_pattern_matching)]
            if let Some(_) = inner.registered_conn_map.get(&rem_id) {
                // TODO : conn.answer(rem_id, data);
                drop(data);
            }
            (Some(inner), Ok(()))
        }));
        Ok(())
    }

    /// Receive an incoming ice candidate from a remote.
    pub fn ice(&self, rem_id: Id, data: Buf) -> Result<()> {
        // not super atomic, but a best effort : )
        if self.0.is_closed() {
            return Err(Error::id("Closed"));
        }
        tokio::task::spawn(self.0.exec(move |inner| async move {
            #[allow(clippy::redundant_pattern_matching)]
            if let Some(_) = inner.registered_conn_map.get(&rem_id) {
                // TODO : conn.ice(rem_id, data);
                drop(data);
            }
            (Some(inner), Ok(()))
        }));
        Ok(())
    }

    // -- //

    pub(crate) fn new(
        state: StateWeak,
        sig_url: Tx4Url,
        maybe_resp: Option<tokio::sync::oneshot::Sender<Result<()>>>,
    ) -> (Self, ManyRcv<Result<SigStateEvt>>) {
        let (sig_snd, sig_rcv) = tokio::sync::mpsc::unbounded_channel();
        let pending_sig_resp = if let Some(resp) = maybe_resp {
            vec![resp]
        } else {
            Vec::new()
        };
        let actor = Actor::new(|this| SigStateData {
            this: SigStateWeak(this),
            state,
            sig_url,
            sig_evt: SigStateEvtSnd(sig_snd),
            connected: false,
            pending_sig_resp,
            registered_conn_map: HashMap::new(),
        });
        let weak = SigStateWeak(actor.weak());
        tokio::task::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(20)).await;
            if let Some(actor) = weak.upgrade() {
                actor.check_connected_timeout().await;
            }
        });
        (Self(actor), ManyRcv(sig_rcv))
    }

    async fn check_connected_timeout(&self) {
        let _ = self
            .0
            .exec(move |mut inner| async move {
                if !inner.connected {
                    inner.shutdown(Error::id("Timeout"));
                }
                (None, Ok(()))
            })
            .await;
    }

    #[allow(dead_code)]
    async fn register_conn(
        &self,
        rem_id: Id,
        conn: ConnStateWeak,
    ) -> Result<()> {
        self.0
            .exec(move |mut inner| async move {
                inner.registered_conn_map.insert(rem_id, conn);
                (Some(inner), Ok(()))
            })
            .await
    }

    pub(crate) fn unregister_conn(&self, rem_id: Id, conn: ConnStateWeak) {
        let fut = self.0.exec(move |mut inner| async move {
            if let Some(oth_conn) = inner.registered_conn_map.remove(&rem_id) {
                if oth_conn != conn {
                    // Whoops!
                    inner.registered_conn_map.insert(rem_id, oth_conn);
                }
            }
            (Some(inner), Ok(()))
        });
        tokio::task::spawn(fut);
    }

    pub(crate) async fn push_assert_respond(
        &self,
        resp: tokio::sync::oneshot::Sender<Result<()>>,
    ) {
        let _ = self
            .0
            .exec(move |mut inner| async move {
                if inner.connected {
                    let _ = resp.send(Ok(()));
                } else {
                    inner.pending_sig_resp.push(resp);
                }
                (Some(inner), Ok(()))
            })
            .await;
    }

    async fn notify_connected(&self, cli_url: Tx4Url) -> Result<()> {
        self.0
            .exec(move |mut inner| async move {
                let r = {
                    let inner = &mut inner;
                    async move {
                        inner.connected = true;
                        for resp in inner.pending_sig_resp.drain(..) {
                            let _ = resp.send(Ok(()));
                        }
                        if let Some(state) = inner.state.upgrade() {
                            state.publish(StateEvt::Address(cli_url)).await?;
                        }
                        Ok(())
                    }
                }
                .await;
                (Some(inner), r)
            })
            .await
    }
}

/*
/// A handle for notifying the state system of signal events.
pub struct SigState {
    inner: StateData,
    key: Key,
    sig_url: Tx4Url,
}

impl SigState {
    /// Shutdown the signal client with an optional error.
    pub fn shutdown(&self, err: std::io::Error) {
        // might need to not use the "new" listener err fn
        // if the side effects are ever incorrect.
        self.inner
            .new_listener_sig_err(self.key, self.sig_url.clone(), err);
    }

    /// Receive an incoming offer from a remote.
    pub fn offer(&self, _rem_id: Id, _data: Buf) -> Result<()> {
        /*
        self.inner.check_incoming_offer(
            &self.sig_url,
            id,
            data,
        )
        */
        todo!()
    }

    /// Receive an incoming answer from a remote.
    pub fn answer(&self, rem_id: Id, data: Buf) {
        self.inner
            .check_incoming_answer(self.sig_url.clone(), rem_id, data)
    }

    /// Receive an incoming ice candidate from a remote.
    pub fn ice(&self, _rem_id: Id, _data: Buf) -> Result<()> {
        todo!()
    }
}
*/
