use super::*;

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
        cli_url: Tx5Url,
        ice_servers: Arc<serde_json::Value>,
    ) -> Result<(SigState, ManyRcv<SigStateEvt>)> {
        self.done = true;
        let (sig, sig_evt) = self.output.take().unwrap();
        sig.notify_connected(cli_url, ice_servers)?;
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
    SndOffer(Id, BackBuf, OneSnd<()>),

    /// Forward an answer to a remote.
    SndAnswer(Id, BackBuf, OneSnd<()>),

    /// Forward an ICE candidate to a remote.
    SndIce(Id, BackBuf, OneSnd<()>),

    /// Trigger a demo broadcast.
    SndDemo,
}

impl std::fmt::Debug for SigStateEvt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SigStateEvt::SndOffer(_, _, _) => f.write_str("SndOffer"),
            SigStateEvt::SndAnswer(_, _, _) => f.write_str("SndAnswer"),
            SigStateEvt::SndIce(_, _, _) => f.write_str("SndIce"),
            SigStateEvt::SndDemo => f.write_str("SndDemo"),
        }
    }
}

#[derive(Clone)]
struct SigStateEvtSnd(tokio::sync::mpsc::UnboundedSender<Result<SigStateEvt>>);

impl SigStateEvtSnd {
    pub fn err(&self, err: std::io::Error) {
        let _ = self.0.send(Err(err));
    }

    pub fn snd_offer(
        &self,
        state: StateWeak,
        sig: SigStateWeak,
        rem_id: Id,
        mut offer: BackBuf,
    ) {
        if let Some(state) = state.upgrade() {
            state.track_sig(rem_id, "offer_out", offer.len().unwrap());
        }
        let s = OneSnd::new(move |result| {
            if let Err(err) = result {
                if let Some(sig) = sig.upgrade() {
                    sig.close(err);
                }
            }
        });
        let _ = self.0.send(Ok(SigStateEvt::SndOffer(rem_id, offer, s)));
    }

    pub fn snd_answer(
        &self,
        state: StateWeak,
        sig: SigStateWeak,
        rem_id: Id,
        mut answer: BackBuf,
    ) {
        if let Some(state) = state.upgrade() {
            state.track_sig(rem_id, "answer_out", answer.len().unwrap());
        }
        let s = OneSnd::new(move |result| {
            if let Err(err) = result {
                if let Some(sig) = sig.upgrade() {
                    sig.close(err);
                }
            }
        });
        let _ = self.0.send(Ok(SigStateEvt::SndAnswer(rem_id, answer, s)));
    }

    pub fn snd_ice(
        &self,
        state: StateWeak,
        sig: SigStateWeak,
        rem_id: Id,
        mut ice: BackBuf,
    ) {
        if let Some(state) = state.upgrade() {
            state.track_sig(rem_id, "ice_out", ice.len().unwrap());
        }
        let s = OneSnd::new(move |result| {
            if let Err(err) = result {
                if let Some(sig) = sig.upgrade() {
                    sig.close(err);
                }
            }
        });
        let _ = self.0.send(Ok(SigStateEvt::SndIce(rem_id, ice, s)));
    }

    pub fn snd_demo(&self) {
        let _ = self.0.send(Ok(SigStateEvt::SndDemo));
    }
}

struct SigStateData {
    this: SigStateWeak,
    state: StateWeak,
    sig_url: Tx5Url,
    sig_evt: SigStateEvtSnd,
    connected: bool,
    cli_url: Option<Tx5Url>,
    ice_servers: Option<Arc<serde_json::Value>>,
    pending_sig_resp: Vec<tokio::sync::oneshot::Sender<Result<Tx5Url>>>,
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
            SigCmd::NotifyConnected {
                cli_url,
                ice_servers,
            } => self.notify_connected(cli_url, ice_servers).await,
            SigCmd::RegisterConn { rem_id, conn, resp } => {
                self.register_conn(rem_id, conn, resp).await
            }
            SigCmd::UnregisterConn { rem_id, conn } => {
                self.unregister_conn(rem_id, conn).await
            }
            SigCmd::Offer { rem_id, data } => self.offer(rem_id, data).await,
            SigCmd::Answer { rem_id, data } => self.answer(rem_id, data).await,
            SigCmd::Ice { rem_id, data } => self.ice(rem_id, data).await,
            SigCmd::Demo { rem_id } => self.demo(rem_id).await,
            SigCmd::SndOffer { rem_id, data } => {
                self.snd_offer(rem_id, data).await
            }
            SigCmd::SndAnswer { rem_id, data } => {
                self.snd_answer(rem_id, data).await
            }
            SigCmd::SndIce { rem_id, data } => self.snd_ice(rem_id, data).await,
            SigCmd::SndDemo => self.snd_demo().await,
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
        resp: tokio::sync::oneshot::Sender<Result<Tx5Url>>,
    ) -> Result<()> {
        if self.connected {
            let _ = resp.send(Ok(self.cli_url.clone().unwrap()));
        } else {
            self.pending_sig_resp.push(resp);
        }
        Ok(())
    }

    async fn notify_connected(
        &mut self,
        cli_url: Tx5Url,
        ice_servers: Arc<serde_json::Value>,
    ) -> Result<()> {
        self.connected = true;
        self.cli_url = Some(cli_url.clone());
        self.ice_servers = Some(ice_servers);
        for resp in self.pending_sig_resp.drain(..) {
            let _ = resp.send(Ok(cli_url.clone()));
        }
        if let Some(state) = self.state.upgrade() {
            state.sig_connected(cli_url);
        }
        Ok(())
    }

    async fn register_conn(
        &mut self,
        rem_id: Id,
        conn: ConnStateWeak,
        resp: tokio::sync::oneshot::Sender<Result<Arc<serde_json::Value>>>,
    ) -> Result<()> {
        self.registered_conn_map.insert(rem_id, conn);
        let _ = resp.send(
            self.ice_servers
                .clone()
                .ok_or_else(|| Error::id("NoIceServers")),
        );
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

    async fn offer(&mut self, rem_id: Id, mut data: BackBuf) -> Result<()> {
        let len = data.len().unwrap();
        if let Some(state) = self.state.upgrade() {
            state.in_offer(self.sig_url.clone(), rem_id, data)?;
            state.track_sig(rem_id, "offer_in", len);
        }
        Ok(())
    }

    async fn answer(&mut self, rem_id: Id, mut data: BackBuf) -> Result<()> {
        let len = data.len().unwrap();
        if let Some(conn) = self.registered_conn_map.get(&rem_id) {
            if let Some(conn) = conn.upgrade() {
                conn.in_answer(data);
            }
        }
        if let Some(state) = self.state.upgrade() {
            state.track_sig(rem_id, "answer_in", len);
        }
        Ok(())
    }

    async fn ice(&mut self, rem_id: Id, data: BackBuf) -> Result<()> {
        if let Some(conn) = self.registered_conn_map.get(&rem_id) {
            if let Some(conn) = conn.upgrade() {
                conn.in_ice(data, true);
                return Ok(());
            }
        }
        if let Some(state) = self.state.upgrade() {
            let _ = state.cache_ice(rem_id, data);
        }
        Ok(())
    }

    async fn demo(&mut self, rem_id: Id) -> Result<()> {
        if let Some(state) = self.state.upgrade() {
            state.in_demo(self.sig_url.clone(), rem_id)?;
        }
        Ok(())
    }

    async fn snd_offer(&mut self, rem_id: Id, data: BackBuf) -> Result<()> {
        self.sig_evt.snd_offer(
            self.state.clone(),
            self.this.clone(),
            rem_id,
            data,
        );
        Ok(())
    }

    async fn snd_answer(&mut self, rem_id: Id, data: BackBuf) -> Result<()> {
        self.sig_evt.snd_answer(
            self.state.clone(),
            self.this.clone(),
            rem_id,
            data,
        );
        Ok(())
    }

    async fn snd_ice(&mut self, rem_id: Id, data: BackBuf) -> Result<()> {
        self.sig_evt.snd_ice(
            self.state.clone(),
            self.this.clone(),
            rem_id,
            data,
        );
        Ok(())
    }

    async fn snd_demo(&mut self) -> Result<()> {
        self.sig_evt.snd_demo();
        Ok(())
    }
}

enum SigCmd {
    CheckConnectedTimeout,
    PushAssertRespond {
        resp: tokio::sync::oneshot::Sender<Result<Tx5Url>>,
    },
    NotifyConnected {
        cli_url: Tx5Url,
        ice_servers: Arc<serde_json::Value>,
    },
    RegisterConn {
        rem_id: Id,
        conn: ConnStateWeak,
        resp: tokio::sync::oneshot::Sender<Result<Arc<serde_json::Value>>>,
    },
    UnregisterConn {
        rem_id: Id,
        conn: ConnStateWeak,
    },
    Offer {
        rem_id: Id,
        data: BackBuf,
    },
    Answer {
        rem_id: Id,
        data: BackBuf,
    },
    Ice {
        rem_id: Id,
        data: BackBuf,
    },
    Demo {
        rem_id: Id,
    },
    SndOffer {
        rem_id: Id,
        data: BackBuf,
    },
    SndAnswer {
        rem_id: Id,
        data: BackBuf,
    },
    SndIce {
        rem_id: Id,
        data: BackBuf,
    },
    SndDemo,
}

async fn sig_state_task(
    mut rcv: ManyRcv<SigCmd>,
    this: SigStateWeak,
    state: StateWeak,
    sig_url: Tx5Url,
    sig_evt: SigStateEvtSnd,
    pending_sig_resp: Vec<tokio::sync::oneshot::Sender<Result<Tx5Url>>>,
) -> Result<()> {
    let mut data = SigStateData {
        this,
        state,
        sig_url,
        sig_evt,
        connected: false,
        cli_url: None,
        ice_servers: None,
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
    pub fn offer(&self, rem_id: Id, data: BackBuf) -> Result<()> {
        self.0.send(Ok(SigCmd::Offer { rem_id, data }))
    }

    /// Receive an incoming answer from a remote.
    pub fn answer(&self, rem_id: Id, data: BackBuf) -> Result<()> {
        self.0.send(Ok(SigCmd::Answer { rem_id, data }))
    }

    /// Receive an incoming ice candidate from a remote.
    pub fn ice(&self, rem_id: Id, data: BackBuf) -> Result<()> {
        self.0.send(Ok(SigCmd::Ice { rem_id, data }))
    }

    /// Receive a demo broadcast from a remote.
    pub fn demo(&self, rem_id: Id) -> Result<()> {
        self.0.send(Ok(SigCmd::Demo { rem_id }))
    }

    // -- //

    pub(crate) fn new(
        state: StateWeak,
        sig_url: Tx5Url,
        resp: tokio::sync::oneshot::Sender<Result<Tx5Url>>,
        timeout: std::time::Duration,
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
            tokio::time::sleep(timeout).await;
            if let Some(actor) = weak.upgrade() {
                actor.check_connected_timeout().await;
            }
        });
        (Self(actor), ManyRcv(sig_rcv))
    }

    pub(crate) fn snd_offer(&self, rem_id: Id, data: BackBuf) -> Result<()> {
        self.0.send(Ok(SigCmd::SndOffer { rem_id, data }))
    }

    pub(crate) fn snd_answer(&self, rem_id: Id, data: BackBuf) -> Result<()> {
        self.0.send(Ok(SigCmd::SndAnswer { rem_id, data }))
    }

    pub(crate) fn snd_ice(&self, rem_id: Id, data: BackBuf) -> Result<()> {
        self.0.send(Ok(SigCmd::SndIce { rem_id, data }))
    }

    pub(crate) fn snd_demo(&self) {
        let _ = self.0.send(Ok(SigCmd::SndDemo));
    }

    async fn check_connected_timeout(&self) {
        let _ = self.0.send(Ok(SigCmd::CheckConnectedTimeout));
    }

    pub(crate) async fn register_conn(
        &self,
        rem_id: Id,
        conn: ConnStateWeak,
    ) -> Result<Arc<serde_json::Value>> {
        let (s, r) = tokio::sync::oneshot::channel();
        self.0.send(Ok(SigCmd::RegisterConn {
            rem_id,
            conn,
            resp: s,
        }))?;
        r.await.map_err(|_| Error::id("Closed"))?
    }

    pub(crate) fn unregister_conn(&self, rem_id: Id, conn: ConnStateWeak) {
        let _ = self.0.send(Ok(SigCmd::UnregisterConn { rem_id, conn }));
    }

    pub(crate) async fn push_assert_respond(
        &self,
        resp: tokio::sync::oneshot::Sender<Result<Tx5Url>>,
    ) {
        let _ = self.0.send(Ok(SigCmd::PushAssertRespond { resp }));
    }

    fn notify_connected(
        &self,
        cli_url: Tx5Url,
        ice_servers: Arc<serde_json::Value>,
    ) -> Result<()> {
        self.0.send(Ok(SigCmd::NotifyConnected {
            cli_url,
            ice_servers,
        }))
    }
}
