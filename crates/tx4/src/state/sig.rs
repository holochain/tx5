use super::*;
//use std::collections::HashMap;

/// Temporary indicating we want a new signal instance.
pub struct SigStateSeed {
    done: bool,
    output: Option<(SigState, ManyRcv<SigStateEvt>)>,
}

impl std::fmt::Debug for SigStateSeed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SigStateSeed").finish()
    }
}

impl Drop for SigStateSeed {
    fn drop(&mut self) {
        self.result_err_inner(Error::id("Dropped"));
    }
}

impl SigStateSeed {
    /// Finalize this sig_state seed by indicating a successful sig connection.
    pub fn result_ok(
        mut self,
        cli_url: Tx4Url,
    ) -> Result<(SigState, ManyRcv<SigStateEvt>)> {
        self.done = true;
        let (sig, sig_evt) = self.output.take().unwrap();
        sig.notify_connected(cli_url)?;
        Ok((sig, sig_evt))
    }

    /// Finalize this sig_state seed by indicating an error connecting.
    pub fn result_err(mut self, err: std::io::Error) {
        self.result_err_inner(err);
    }

    // -- //

    pub(crate) fn new(sig: SigState, sig_evt: ManyRcv<SigStateEvt>) -> Self {
        Self {
            done: false,
            output: Some((sig, sig_evt)),
        }
    }

    fn result_err_inner(&mut self, err: std::io::Error) {
        if !self.done {
            self.done = true;
            if let Some((sig, _)) = self.output.take() {
                sig.close(err);
            }
        }
    }
}

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

    pub fn snd_offer(&self, sig: SigStateWeak, rem_id: Id, offer: Buf) {
        let s = OneSnd::new(move |result| {
            if let Err(err) = result {
                if let Some(sig) = sig.upgrade() {
                    sig.close(err);
                }
            }
        });
        let _ = self.0.send(Ok(SigStateEvt::SndOffer(rem_id, offer, s)));
    }

    pub fn snd_ice(&self, sig: SigStateWeak, rem_id: Id, ice: Buf) {
        let s = OneSnd::new(move |result| {
            if let Err(err) = result {
                if let Some(sig) = sig.upgrade() {
                    sig.close(err);
                }
            }
        });
        let _ = self.0.send(Ok(SigStateEvt::SndIce(rem_id, ice, s)));
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

    async fn exec(&mut self, cmd: SigCmd) -> Result<()> {
        match cmd {
            SigCmd::CheckConnectedTimeout => {
                self.check_connected_timeout().await
            }
            SigCmd::PushAssertRespond { resp } => {
                self.push_assert_respond(resp).await
            }
            SigCmd::NotifyConnected { cli_url } => {
                self.notify_connected(cli_url).await
            }
            SigCmd::RegisterConn { rem_id, conn } => {
                self.register_conn(rem_id, conn).await
            }
            SigCmd::UnregisterConn { rem_id, conn } => {
                self.unregister_conn(rem_id, conn).await
            }
            SigCmd::Offer { rem_id, data } => self.offer(rem_id, data).await,
            SigCmd::Answer { rem_id, data } => self.answer(rem_id, data).await,
            SigCmd::Ice { rem_id, data } => self.ice(rem_id, data).await,
            SigCmd::SndOffer { rem_id, data } => {
                self.snd_offer(rem_id, data).await
            }
            SigCmd::SndIce { rem_id, data } => self.snd_ice(rem_id, data).await,
        }
    }

    async fn check_connected_timeout(&mut self) -> Result<()> {
        if !self.connected {
            Err(Error::id("Timeout"))
        } else {
            Ok(())
        }
    }

    async fn push_assert_respond(
        &mut self,
        resp: tokio::sync::oneshot::Sender<Result<()>>,
    ) -> Result<()> {
        if self.connected {
            let _ = resp.send(Ok(()));
        } else {
            self.pending_sig_resp.push(resp);
        }
        Ok(())
    }

    async fn notify_connected(&mut self, cli_url: Tx4Url) -> Result<()> {
        self.connected = true;
        for resp in self.pending_sig_resp.drain(..) {
            let _ = resp.send(Ok(()));
        }
        if let Some(state) = self.state.upgrade() {
            state.publish(StateEvt::Address(cli_url));
        }
        Ok(())
    }

    async fn register_conn(
        &mut self,
        rem_id: Id,
        conn: ConnStateWeak,
    ) -> Result<()> {
        self.registered_conn_map.insert(rem_id, conn);
        Ok(())
    }

    async fn unregister_conn(
        &mut self,
        rem_id: Id,
        conn: ConnStateWeak,
    ) -> Result<()> {
        if let Some(cur_conn) = self.registered_conn_map.remove(&rem_id) {
            if cur_conn != conn {
                // Whoops!
                self.registered_conn_map.insert(rem_id, cur_conn);
            }
        }
        Ok(())
    }

    async fn offer(&mut self, _rem_id: Id, _data: Buf) -> Result<()> {
        todo!()
    }

    async fn answer(&mut self, rem_id: Id, data: Buf) -> Result<()> {
        if let Some(conn) = self.registered_conn_map.get(&rem_id) {
            if let Some(conn) = conn.upgrade() {
                conn.in_answer(data);
            }
        }
        Ok(())
    }

    async fn ice(&mut self, rem_id: Id, data: Buf) -> Result<()> {
        if let Some(conn) = self.registered_conn_map.get(&rem_id) {
            if let Some(conn) = conn.upgrade() {
                conn.in_ice(data);
            }
        }
        Ok(())
    }

    async fn snd_offer(&mut self, rem_id: Id, data: Buf) -> Result<()> {
        self.sig_evt.snd_offer(self.this.clone(), rem_id, data);
        Ok(())
    }

    async fn snd_ice(&mut self, rem_id: Id, data: Buf) -> Result<()> {
        self.sig_evt.snd_ice(self.this.clone(), rem_id, data);
        Ok(())
    }
}

enum SigCmd {
    CheckConnectedTimeout,
    PushAssertRespond {
        resp: tokio::sync::oneshot::Sender<Result<()>>,
    },
    NotifyConnected {
        cli_url: Tx4Url,
    },
    RegisterConn {
        rem_id: Id,
        conn: ConnStateWeak,
    },
    UnregisterConn {
        rem_id: Id,
        conn: ConnStateWeak,
    },
    Offer {
        rem_id: Id,
        data: Buf,
    },
    Answer {
        rem_id: Id,
        data: Buf,
    },
    Ice {
        rem_id: Id,
        data: Buf,
    },
    SndOffer {
        rem_id: Id,
        data: Buf,
    },
    SndIce {
        rem_id: Id,
        data: Buf,
    },
}

async fn sig_state_task(
    mut rcv: ManyRcv<SigCmd>,
    this: SigStateWeak,
    state: StateWeak,
    sig_url: Tx4Url,
    sig_evt: SigStateEvtSnd,
    pending_sig_resp: Vec<tokio::sync::oneshot::Sender<Result<()>>>,
) -> Result<()> {
    let mut data = SigStateData {
        this,
        state,
        sig_url,
        sig_evt,
        connected: false,
        pending_sig_resp,
        registered_conn_map: HashMap::new(),
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

/// Weak version of SigState.
#[derive(Clone, PartialEq, Eq)]
pub struct SigStateWeak(ActorWeak<SigCmd>);

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
pub struct SigState(Actor<SigCmd>);

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
        self.0.close(err);
    }

    /// Receive an incoming offer from a remote.
    pub fn offer(&self, rem_id: Id, data: Buf) -> Result<()> {
        self.0.send(Ok(SigCmd::Offer { rem_id, data }))
    }

    /// Receive an incoming answer from a remote.
    pub fn answer(&self, rem_id: Id, data: Buf) -> Result<()> {
        self.0.send(Ok(SigCmd::Answer { rem_id, data }))
    }

    /// Receive an incoming ice candidate from a remote.
    pub fn ice(&self, rem_id: Id, data: Buf) -> Result<()> {
        self.0.send(Ok(SigCmd::Ice { rem_id, data }))
    }

    // -- //

    pub(crate) fn new(
        state: StateWeak,
        sig_url: Tx4Url,
        resp: tokio::sync::oneshot::Sender<Result<()>>,
    ) -> (Self, ManyRcv<SigStateEvt>) {
        let (sig_snd, sig_rcv) = tokio::sync::mpsc::unbounded_channel();
        let actor = Actor::new(move |this, rcv| {
            sig_state_task(
                rcv,
                SigStateWeak(this),
                state,
                sig_url,
                SigStateEvtSnd(sig_snd),
                vec![resp],
            )
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

    pub(crate) fn snd_offer(&self, rem_id: Id, data: Buf) -> Result<()> {
        self.0.send(Ok(SigCmd::SndOffer { rem_id, data }))
    }

    pub(crate) fn snd_ice(&self, rem_id: Id, data: Buf) -> Result<()> {
        self.0.send(Ok(SigCmd::SndIce { rem_id, data }))
    }

    async fn check_connected_timeout(&self) {
        let _ = self.0.send(Ok(SigCmd::CheckConnectedTimeout));
    }

    pub(crate) fn register_conn(
        &self,
        rem_id: Id,
        conn: ConnStateWeak,
    ) -> Result<()> {
        self.0.send(Ok(SigCmd::RegisterConn { rem_id, conn }))
    }

    pub(crate) fn unregister_conn(&self, rem_id: Id, conn: ConnStateWeak) {
        let _ = self.0.send(Ok(SigCmd::UnregisterConn { rem_id, conn }));
    }

    pub(crate) async fn push_assert_respond(
        &self,
        resp: tokio::sync::oneshot::Sender<Result<()>>,
    ) {
        let _ = self.0.send(Ok(SigCmd::PushAssertRespond { resp }));
    }

    fn notify_connected(&self, cli_url: Tx4Url) -> Result<()> {
        self.0.send(Ok(SigCmd::NotifyConnected { cli_url }))
    }
}
