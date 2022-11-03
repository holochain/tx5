use super::*;
use std::sync::atomic;

/// Temporary indicating we want a new conn instance.
pub struct ConnStateSeed {
    done: bool,
    output: Option<(ConnState, ManyRcv<ConnStateEvt>)>,
}

impl std::fmt::Debug for ConnStateSeed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnStateSeed").finish()
    }
}

impl Drop for ConnStateSeed {
    fn drop(&mut self) {
        self.result_err_inner(Error::id("Dropped"));
    }
}

impl ConnStateSeed {
    /// Finalize this conn_state seed by indicating a successful connection.
    pub fn result_ok(mut self) -> Result<(ConnState, ManyRcv<ConnStateEvt>)> {
        self.done = true;
        let (conn, conn_evt) = self.output.take().unwrap();
        conn.notify_constructed()?;
        Ok((conn, conn_evt))
    }

    /// Finalize this conn_state seed by indicating an error connecting.
    pub fn result_err(mut self, err: std::io::Error) {
        self.result_err_inner(err);
    }

    // -- //

    pub(crate) fn new(
        conn: ConnState,
        conn_evt: ManyRcv<ConnStateEvt>,
    ) -> Self {
        Self {
            done: false,
            output: Some((conn, conn_evt)),
        }
    }

    fn result_err_inner(&mut self, err: std::io::Error) {
        if !self.done {
            self.done = true;
            if let Some((conn, _)) = self.output.take() {
                conn.close(err);
            }
        }
    }
}

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
    CreateOffer(OneSnd<Buf>),

    /// Request to create an answer.
    CreateAnswer(OneSnd<Buf>),

    /// Request to set a local description.
    SetLoc(Buf, OneSnd<()>),

    /// Request to set a remote description.
    SetRem(Buf, OneSnd<()>),

    /// Request to append a trickle ICE candidate.
    SetIce(Buf, OneSnd<()>),

    /// Request to send a message on the data channel.
    SndData(Buf, OneSnd<BufState>),
}

impl std::fmt::Debug for ConnStateEvt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConnStateEvt::CreateOffer(_) => f.write_str("CreateOffer"),
            ConnStateEvt::CreateAnswer(_) => f.write_str("CreateAnswer"),
            ConnStateEvt::SetLoc(_, _) => f.write_str("SetLoc"),
            ConnStateEvt::SetRem(_, _) => f.write_str("SetRem"),
            ConnStateEvt::SetIce(_, _) => f.write_str("SetIce"),
            ConnStateEvt::SndData(_, _) => f.write_str("SndData"),
        }
    }
}

#[derive(Clone)]
struct ConnStateEvtSnd(
    tokio::sync::mpsc::UnboundedSender<Result<ConnStateEvt>>,
);

impl ConnStateEvtSnd {
    pub fn err(&self, err: std::io::Error) {
        let _ = self.0.send(Err(err));
    }

    pub fn create_offer(&self, conn: ConnStateWeak) {
        let s = OneSnd::new(move |result| {
            if let Some(conn) = conn.upgrade() {
                conn.self_offer(result);
            }
        });
        let _ = self.0.send(Ok(ConnStateEvt::CreateOffer(s)));
    }

    pub fn create_answer(&self, conn: ConnStateWeak) {
        let s = OneSnd::new(move |result| {
            if let Some(conn) = conn.upgrade() {
                conn.self_answer(result);
            }
        });
        let _ = self.0.send(Ok(ConnStateEvt::CreateAnswer(s)));
    }

    pub fn set_loc(&self, conn: ConnStateWeak, data: Buf) {
        let s = OneSnd::new(move |result| {
            if let Err(err) = result {
                if let Some(conn) = conn.upgrade() {
                    conn.close(err);
                }
            }
        });
        let _ = self.0.send(Ok(ConnStateEvt::SetLoc(data, s)));
    }

    pub fn set_rem(&self, conn: ConnStateWeak, data: Buf, should_answer: bool) {
        let s = if should_answer {
            OneSnd::new(move |result| match result {
                Ok(_) => {
                    if let Some(conn) = conn.upgrade() {
                        conn.req_self_answer();
                    }
                }
                Err(err) => {
                    if let Some(conn) = conn.upgrade() {
                        conn.close(err);
                    }
                }
            })
        } else {
            OneSnd::new(move |result| {
                if let Err(err) = result {
                    if let Some(conn) = conn.upgrade() {
                        conn.close(err);
                    }
                }
            })
        };
        let _ = self.0.send(Ok(ConnStateEvt::SetRem(data, s)));
    }

    pub fn set_ice(&self, _conn: ConnStateWeak, data: Buf) {
        let s = OneSnd::new(move |result| {
            if let Err(err) = result {
                tracing::debug!(?err, "ICEError");
                // treat ice errors loosely... sometimes things
                // get out of order... especially with perfect negotiation
                /*
                if let Some(conn) = conn.upgrade() {
                    conn.close(err);
                }
                */
            }
        });
        let _ = self.0.send(Ok(ConnStateEvt::SetIce(data, s)));
    }

    pub fn snd_data(
        &self,
        conn: ConnStateWeak,
        data: Buf,
        resp: Option<tokio::sync::oneshot::Sender<Result<()>>>,
        send_permit: tokio::sync::OwnedSemaphorePermit,
    ) {
        let s = OneSnd::new(move |result| {
            let _send_permit = send_permit;
            match result {
                Err(err) => {
                    if let Some(conn) = conn.upgrade() {
                        conn.close(err.err_clone());
                    }
                    if let Some(resp) = resp {
                        let _ = resp.send(Err(err));
                    }
                }
                Ok(buffer_state) => {
                    if let Some(conn) = conn.upgrade() {
                        conn.set_buffer_state(buffer_state);
                    }
                    if let Some(resp) = resp {
                        let _ = resp.send(Ok(()));
                    }
                }
            }
        });
        let _ = self.0.send(Ok(ConnStateEvt::SndData(data, s)));
    }
}

struct ConnStateData {
    this: ConnStateWeak,
    metrics: prometheus::Registry,
    metric_conn_count: prometheus::IntGauge,
    meta: ConnStateMeta,
    state: StateWeak,
    this_id: Id,
    cli_url: Tx4Url,
    rem_id: Id,
    conn_evt: ConnStateEvtSnd,
    sig_state: SigStateWeak,
    rcv_offer: bool,
}

impl Drop for ConnStateData {
    fn drop(&mut self) {
        self.shutdown(Error::id("Dropped"));
    }
}

impl ConnStateData {
    fn connected(&self) -> bool {
        self.meta.connected.load(atomic::Ordering::SeqCst)
    }

    fn shutdown(&mut self, err: std::io::Error) {
        tracing::debug!(
            ?err,
            this_id = ?self.this_id,
            rem_id = ?self.rem_id,
            "ConnShutdown",
        );
        self.metric_conn_count.dec();
        let _ = self
            .metrics
            .unregister(Box::new(self.meta.metric_bytes_snd.clone()));
        let _ = self
            .metrics
            .unregister(Box::new(self.meta.metric_bytes_rcv.clone()));
        let _ = self
            .metrics
            .unregister(Box::new(self.meta.metric_age.clone()));
        let _ = self
            .metrics
            .unregister(Box::new(self.meta.metric_last_active.clone()));
        if let Some(state) = self.state.upgrade() {
            state.close_conn(self.rem_id, self.this.clone(), err.err_clone());
        }
        if let Some(sig) = self.sig_state.upgrade() {
            sig.unregister_conn(self.rem_id, self.this.clone());
        }
        self.conn_evt.err(err);
    }

    fn get_sig(&mut self) -> Result<SigState> {
        match self.sig_state.upgrade() {
            Some(sig) => Ok(sig),
            None => Err(Error::id("SigClosed")),
        }
    }

    async fn exec(&mut self, cmd: ConnCmd) -> Result<()> {
        match cmd {
            ConnCmd::Tick1s => self.tick_1s().await,
            ConnCmd::NotifyConstructed => self.notify_constructed().await,
            ConnCmd::CheckConnectedTimeout => {
                self.check_connected_timeout().await
            }
            ConnCmd::Ice { data } => self.ice(data).await,
            ConnCmd::SelfOffer { offer } => self.self_offer(offer).await,
            ConnCmd::ReqSelfAnswer => self.req_self_answer().await,
            ConnCmd::SelfAnswer { answer } => self.self_answer(answer).await,
            ConnCmd::InOffer { offer } => self.in_offer(offer).await,
            ConnCmd::InAnswer { answer } => self.in_answer(answer).await,
            ConnCmd::InIce { ice, cache } => self.in_ice(ice, cache).await,
            ConnCmd::Ready => self.ready().await,
            ConnCmd::MaybeFetchForSend => self.maybe_fetch_for_send().await,
            ConnCmd::Send { to_send } => self.send(to_send).await,
            ConnCmd::Recv { data, permit } => self.recv(data, permit).await,
        }
    }

    async fn tick_1s(&mut self) -> Result<()> {
        if self.meta.metric_last_active.elapsed()
            > std::time::Duration::from_secs(20)
            && !self.connected()
        {
            self.shutdown(Error::id("InactivityTimeout"));
        }

        Ok(())
    }

    async fn notify_constructed(&mut self) -> Result<()> {
        if !self.rcv_offer {
            // Kick off connection initialization by requesting
            // an outgoing offer be created by this connection.
            // This will result in a `self_offer` call.
            self.conn_evt.create_offer(self.this.clone());
        }
        Ok(())
    }

    async fn check_connected_timeout(&mut self) -> Result<()> {
        if !self.connected() {
            Err(Error::id("Timeout"))
        } else {
            Ok(())
        }
    }

    async fn ice(&mut self, data: Buf) -> Result<()> {
        let sig = self.get_sig()?;
        sig.snd_ice(self.rem_id, data)
    }

    async fn self_offer(&mut self, offer: Result<Buf>) -> Result<()> {
        let sig = self.get_sig()?;
        let mut offer = offer?;
        self.conn_evt.set_loc(self.this.clone(), offer.try_clone()?);
        sig.snd_offer(self.rem_id, offer)
    }

    async fn req_self_answer(&mut self) -> Result<()> {
        self.conn_evt.create_answer(self.this.clone());
        Ok(())
    }

    async fn self_answer(&mut self, answer: Result<Buf>) -> Result<()> {
        let sig = self.get_sig()?;
        let mut answer = answer?;
        self.conn_evt
            .set_loc(self.this.clone(), answer.try_clone()?);
        sig.snd_answer(self.rem_id, answer)
    }

    async fn in_offer(&mut self, mut offer: Buf) -> Result<()> {
        tracing::trace!(
            this_id = ?self.this_id,
            rem_id = ?self.rem_id,
            offer = %String::from_utf8_lossy(&offer.to_vec()?),
            "OfferRecv",
        );
        self.rcv_offer = true;
        self.conn_evt.set_rem(self.this.clone(), offer, true);
        self.state
            .upgrade()
            .ok_or_else(|| Error::id("Closed"))?
            .get_cached_ice(self.rem_id)?;
        Ok(())
    }

    async fn in_answer(&mut self, mut answer: Buf) -> Result<()> {
        tracing::trace!(
            this_id = ?self.this_id,
            rem_id = ?self.rem_id,
            answer = %String::from_utf8_lossy(&answer.to_vec()?),
            "AnswerRecv",
        );
        self.conn_evt.set_rem(self.this.clone(), answer, false);
        self.state
            .upgrade()
            .ok_or_else(|| Error::id("Closed"))?
            .get_cached_ice(self.rem_id)?;
        Ok(())
    }

    async fn in_ice(&mut self, mut ice: Buf, cache: bool) -> Result<()> {
        tracing::trace!(
            this_id = ?self.this_id,

            rem_id = ?self.rem_id,
            ice = %String::from_utf8_lossy(&ice.to_vec()?),
            "ICERecv",
        );
        if cache {
            self.state
                .upgrade()
                .ok_or_else(|| Error::id("Closed"))?
                .cache_ice(self.rem_id, ice.try_clone()?)?;
        }
        self.conn_evt.set_ice(self.this.clone(), ice);
        Ok(())
    }

    async fn ready(&mut self) -> Result<()> {
        self.meta.connected.store(true, atomic::Ordering::SeqCst);
        self.maybe_fetch_for_send().await
    }

    async fn maybe_fetch_for_send(&mut self) -> Result<()> {
        if !self.connected() {
            return Ok(());
        }

        // if we are within the close time send grace period
        // do not send any new messages so we can try to shut
        // down gracefully
        if self.meta.metric_age.elapsed()
            > (MAX_CON_TIME - CON_CLOSE_SEND_GRACE)
        {
            return Ok(());
        }

        // TODO - also check our buffer amt low status

        if let Some(state) = self.state.upgrade() {
            state.fetch_for_send(self.this.clone(), self.rem_id)?;
            Ok(())
        } else {
            Err(Error::id("StateClosed"))
        }
    }

    async fn send(&mut self, to_send: SendData) -> Result<()> {
        self.meta.metric_last_active.reset();

        let SendData {
            mut data,
            resp,
            send_permit,
            ..
        } = to_send;

        self.meta.metric_bytes_snd.inc_by(data.len()? as u64);

        self.conn_evt
            .snd_data(self.this.clone(), data, resp, send_permit);

        Ok(())
    }

    async fn recv(
        &mut self,
        mut data: Buf,
        permit: tokio::sync::OwnedSemaphorePermit,
    ) -> Result<()> {
        self.meta.metric_last_active.reset();

        self.meta.metric_bytes_rcv.inc_by(data.len()? as u64);

        if let Some(state) = self.state.upgrade() {
            state.publish(StateEvt::RcvData(
                self.cli_url.clone(),
                data,
                Permit(permit),
            ));
        }

        Ok(())
    }
}

enum ConnCmd {
    Tick1s,
    NotifyConstructed,
    CheckConnectedTimeout,
    Ice {
        data: Buf,
    },
    SelfOffer {
        offer: Result<Buf>,
    },
    ReqSelfAnswer,
    SelfAnswer {
        answer: Result<Buf>,
    },
    InOffer {
        offer: Buf,
    },
    InAnswer {
        answer: Buf,
    },
    InIce {
        ice: Buf,
        cache: bool,
    },
    Ready,
    MaybeFetchForSend,
    Send {
        to_send: SendData,
    },
    Recv {
        data: Buf,
        permit: tokio::sync::OwnedSemaphorePermit,
    },
}

#[allow(clippy::too_many_arguments)]
async fn conn_state_task(
    conn_limit: Arc<tokio::sync::Semaphore>,
    metrics: prometheus::Registry,
    metric_conn_count: prometheus::IntGauge,
    meta: ConnStateMeta,
    strong: ConnState,
    conn_rcv: ManyRcv<ConnStateEvt>,
    mut rcv: ManyRcv<ConnCmd>,
    this: ConnStateWeak,
    state: StateWeak,
    this_id: Id,
    cli_url: Tx4Url,
    rem_id: Id,
    conn_evt: ConnStateEvtSnd,
    sig_state: SigStateWeak,
    sig_ready: tokio::sync::oneshot::Receiver<Result<Tx4Url>>,
) -> Result<()> {
    metric_conn_count.inc();

    let mut data = ConnStateData {
        this,
        metrics,
        metric_conn_count,
        meta,
        state,
        this_id,
        cli_url,
        rem_id,
        conn_evt,
        sig_state,
        rcv_offer: false,
    };

    let mut permit = None;

    let err = match async {
        permit = Some(
            conn_limit
                .acquire_owned()
                .await
                .map_err(|_| Error::id("Closed"))?,
        );

        sig_ready.await.map_err(|_| Error::id("SigClosed"))??;

        let sig = data.get_sig()?;
        let ice_servers =
            sig.register_conn(data.rem_id, data.this.clone()).await?;

        match data.state.upgrade() {
            None => return Err(Error::id("Closed")),
            Some(state) => {
                tracing::debug!(id = ?data.rem_id, "NewConn");
                let seed = ConnStateSeed::new(strong, conn_rcv);
                state.publish(StateEvt::NewConn(ice_servers, seed));
            }
        }

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

    drop(permit);

    Err(err)
}

#[derive(Clone)]
pub(crate) struct ConnStateMeta {
    pub(crate) config: DynConfig,
    pub(crate) connected: Arc<atomic::AtomicBool>,
    pub(crate) rcv_limit: Arc<tokio::sync::Semaphore>,
    pub(crate) metric_bytes_snd: prometheus::IntCounter,
    pub(crate) metric_bytes_rcv: prometheus::IntCounter,
    pub(crate) metric_age: MetricTimestamp,
    pub(crate) metric_last_active: MetricTimestamp,
}

/// Weak version on ConnState.
#[derive(Clone)]
pub struct ConnStateWeak(ActorWeak<ConnCmd>, ConnStateMeta);

impl PartialEq for ConnStateWeak {
    fn eq(&self, rhs: &ConnStateWeak) -> bool {
        self.0 == rhs.0
    }
}

impl PartialEq<ConnState> for ConnStateWeak {
    fn eq(&self, rhs: &ConnState) -> bool {
        self.0 == rhs.0
    }
}

impl Eq for ConnStateWeak {}

impl ConnStateWeak {
    /// Access the meta struct
    pub(crate) fn meta(&self) -> &ConnStateMeta {
        &self.1
    }

    /// Upgrade to a full ConnState instance.
    pub fn upgrade(&self) -> Option<ConnState> {
        self.0.upgrade().map(|i| ConnState(i, self.1.clone()))
    }
}

/// A handle for notifying the state system of connection events.
#[derive(Clone)]
pub struct ConnState(Actor<ConnCmd>, ConnStateMeta);

impl PartialEq for ConnState {
    fn eq(&self, rhs: &ConnState) -> bool {
        self.0 == rhs.0
    }
}

impl PartialEq<ConnStateWeak> for ConnState {
    fn eq(&self, rhs: &ConnStateWeak) -> bool {
        self.0 == rhs.0
    }
}

impl Eq for ConnState {}

impl ConnState {
    /*
    /// Access the meta struct
    pub(crate) fn meta(&self) -> &ConnStateMeta {
        &self.1
    }
    */

    /// Get a weak version of this ConnState instance.
    pub fn weak(&self) -> ConnStateWeak {
        ConnStateWeak(self.0.weak(), self.1.clone())
    }

    /// Returns `true` if this ConnState is closed.
    pub fn is_closed(&self) -> bool {
        self.0.is_closed()
    }

    /// Shutdown the connection with an error.
    pub fn close(&self, err: std::io::Error) {
        self.0.close(err);
    }

    /// The connection generated an ice candidate for the remote.
    pub fn ice(&self, data: Buf) -> Result<()> {
        self.0.send(Ok(ConnCmd::Ice { data }))
    }

    /// The connection is ready to send and receive data.
    pub fn ready(&self) -> Result<()> {
        self.0.send(Ok(ConnCmd::Ready))
    }

    /// The connection received data on the data channel.
    /// This synchronous function must not block for now...
    /// (we'll need to test some blocking strategies
    /// for the goroutine in tx4-go-pion)... but we also can't just
    /// fill up memory if the application is processing slowly.
    /// So it will error / trigger connection shutdown if we get
    /// too much of a backlog.
    pub fn rcv_data(&self, data: Buf) -> Result<()> {
        // we've got 15 ms of time to acquire the recv permit
        // this is a little more forgiving than just blanket deny
        // if the app is a little slow, but it's also not so much time
        // that we'll end up stalling other tasks that need to run.

        let mut data = data;
        let len = data.len()?;
        if len > self.1.config.max_recv_bytes() as usize {
            let err = Error::id("DataTooLarge");
            self.close(err.err_clone());
            return Err(err);
        }

        // polling try_acquire doesn't fairly reserve a place in line,
        // so we need to timeout an actual acquire future..

        let fut = tokio::time::timeout(
            std::time::Duration::from_millis(15),
            async move {
                let permit = self
                    .1
                    .rcv_limit
                    .clone()
                    .acquire_many_owned(len as u32)
                    .await
                    .map_err(|_| Error::id("Closed"))?;
                self.0.send(Ok(ConnCmd::Recv { data, permit }))
            },
        );

        // need an external (futures) polling executor, because tokio
        // won't let us use blocking_recv, since we're still in the runtime.

        futures::executor::block_on(fut)
            .map_err(|_| Error::id("RecvQueueFull"))?
    }

    /// The send buffer *was* high, but has now transitioned to low.
    pub fn buf_amt_low(&self) -> Result<()> {
        todo!()
    }

    // -- //

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new_and_publish(
        config: DynConfig,
        conn_limit: Arc<tokio::sync::Semaphore>,
        state_prefix: Arc<str>,
        metrics: prometheus::Registry,
        metric_conn_count: prometheus::IntGauge,
        state: StateWeak,
        sig_state: SigStateWeak,
        this_id: Id,
        cli_url: Tx4Url,
        rem_id: Id,
        rcv_limit: Arc<tokio::sync::Semaphore>,
        sig_ready: tokio::sync::oneshot::Receiver<Result<Tx4Url>>,
        maybe_offer: Option<Buf>,
    ) -> Result<ConnStateWeak> {
        let (conn_snd, conn_rcv) = tokio::sync::mpsc::unbounded_channel();

        let uniq = uniq();

        let metric_bytes_snd = prometheus::IntCounter::new(
            format!("{}_conn_{}_bytes_snd", state_prefix, uniq),
            "bytes sent out of this connection",
        )
        .map_err(Error::err)?;
        metrics
            .register(Box::new(metric_bytes_snd.clone()))
            .map_err(Error::err)?;

        let metric_bytes_rcv = prometheus::IntCounter::new(
            format!("{}_conn_{}_bytes_rcv", state_prefix, uniq),
            "incoming bytes received by this connection",
        )
        .map_err(Error::err)?;
        metrics
            .register(Box::new(metric_bytes_rcv.clone()))
            .map_err(Error::err)?;

        let metric_age = MetricTimestamp::new(
            format!("{}_conn_{}_age", state_prefix, uniq),
            "microseconds since this connection was created",
        )
        .map_err(Error::err)?;
        metrics
            .register(Box::new(metric_age.clone()))
            .map_err(Error::err)?;

        let metric_last_active = MetricTimestamp::new(
            format!("{}_conn_{}_last_active", state_prefix, uniq),
            "microseconds since we last sent or received data on this connection",
        )
        .map_err(Error::err)?;
        metrics
            .register(Box::new(metric_last_active.clone()))
            .map_err(Error::err)?;

        let meta = ConnStateMeta {
            config,
            connected: Arc::new(atomic::AtomicBool::new(false)),
            rcv_limit,
            metric_bytes_snd,
            metric_bytes_rcv,
            metric_age,
            metric_last_active,
        };

        let actor = {
            let meta = meta.clone();
            Actor::new(move |this, rcv| {
                // woo, this is wonkey...
                // we actually publish the "strong" version of this
                // inside the task after waiting for the sig to be ready
                // so upgrade here while the outer strong still exists,
                // then the outer strong will be downgraded to return
                // from new_and_publish...
                let strong = ConnState(this.upgrade().unwrap(), meta.clone());
                conn_state_task(
                    conn_limit,
                    metrics,
                    metric_conn_count,
                    meta.clone(),
                    strong,
                    ManyRcv(conn_rcv),
                    rcv,
                    ConnStateWeak(this, meta),
                    state,
                    this_id,
                    cli_url,
                    rem_id,
                    ConnStateEvtSnd(conn_snd),
                    sig_state,
                    sig_ready,
                )
            })
        };

        let actor = ConnState(actor, meta);

        if let Some(offer) = maybe_offer {
            actor.in_offer(offer);
        }

        let weak = actor.weak();
        tokio::task::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(20)).await;
            if let Some(actor) = weak.upgrade() {
                actor.check_connected_timeout().await;
            }
        });

        let weak = actor.weak();
        tokio::task::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                match weak.upgrade() {
                    None => break,
                    Some(actor) => {
                        if actor.tick_1s().is_err() {
                            break;
                        }
                    }
                }
            }
        });

        Ok(actor.weak())
    }

    fn tick_1s(&self) -> Result<()> {
        self.0.send(Ok(ConnCmd::Tick1s))
    }

    async fn check_connected_timeout(&self) {
        let _ = self.0.send(Ok(ConnCmd::CheckConnectedTimeout));
    }

    fn notify_constructed(&self) -> Result<()> {
        self.0.send(Ok(ConnCmd::NotifyConstructed))
    }

    fn self_offer(&self, offer: Result<Buf>) {
        let _ = self.0.send(Ok(ConnCmd::SelfOffer { offer }));
    }

    fn req_self_answer(&self) {
        let _ = self.0.send(Ok(ConnCmd::ReqSelfAnswer));
    }

    fn self_answer(&self, answer: Result<Buf>) {
        let _ = self.0.send(Ok(ConnCmd::SelfAnswer { answer }));
    }

    pub(crate) fn in_offer(&self, offer: Buf) {
        let _ = self.0.send(Ok(ConnCmd::InOffer { offer }));
    }

    pub(crate) fn in_answer(&self, answer: Buf) {
        let _ = self.0.send(Ok(ConnCmd::InAnswer { answer }));
    }

    pub(crate) fn in_ice(&self, ice: Buf, cache: bool) {
        let _ = self.0.send(Ok(ConnCmd::InIce { ice, cache }));
    }

    pub(crate) async fn notify_send_waiting(&self) {
        let _ = self.0.send(Ok(ConnCmd::MaybeFetchForSend));
    }

    pub(crate) fn send(&self, to_send: SendData) {
        let _ = self.0.send(Ok(ConnCmd::Send { to_send }));
    }

    pub(crate) fn set_buffer_state(&self, _buffer_state: BufState) {}
}
