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
    /// BackBuffer is low, we can buffer more data.
    Low,

    /// BackBuffer is high, we should wait / apply backpressure.
    High,
}

/// State wishes to invoke an action on a connection instance.
pub enum ConnStateEvt {
    /// Request to create an offer.
    CreateOffer(OneSnd<BackBuf>),

    /// Request to create an answer.
    CreateAnswer(OneSnd<BackBuf>),

    /// Request to set a local description.
    SetLoc(BackBuf, OneSnd<()>),

    /// Request to set a remote description.
    SetRem(BackBuf, OneSnd<()>),

    /// Request to append a trickle ICE candidate.
    SetIce(BackBuf, OneSnd<()>),

    /// Request to send a message on the data channel.
    SndData(BackBuf, OneSnd<BufState>),

    /// Request a stats dump of this peer connection.
    Stats(OneSnd<BackBuf>),
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
            ConnStateEvt::Stats(_) => f.write_str("Stats"),
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

    pub fn set_loc(&self, conn: ConnStateWeak, data: BackBuf) {
        let s = OneSnd::new(move |result| {
            if let Err(err) = result {
                if let Some(conn) = conn.upgrade() {
                    conn.close(err);
                }
            }
        });
        let _ = self.0.send(Ok(ConnStateEvt::SetLoc(data, s)));
    }

    pub fn set_rem(
        &self,
        conn: ConnStateWeak,
        data: BackBuf,
        should_answer: bool,
    ) {
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

    pub fn set_ice(&self, _conn: ConnStateWeak, data: BackBuf) {
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
        data: BackBuf,
        resp: Option<tokio::sync::oneshot::Sender<Result<()>>>,
        send_permit: Option<tokio::sync::OwnedSemaphorePermit>,
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
                        conn.notify_send_complete(buffer_state);
                    }
                    if let Some(resp) = resp {
                        let _ = resp.send(Ok(()));
                    }
                }
            }
        });
        let _ = self.0.send(Ok(ConnStateEvt::SndData(data, s)));
    }

    pub fn stats(
        &self,
        additions: Vec<(String, serde_json::Value)>,
        rsp: tokio::sync::oneshot::Sender<Result<serde_json::Value>>,
    ) {
        let _ = self.0.send(Ok(ConnStateEvt::Stats(OneSnd::new(
            move |buf: Result<BackBuf>| {
                let _ = rsp.send((move || {
                    let mut stats: serde_json::Value = buf?.to_json()?;
                    for (key, value) in additions {
                        stats.as_object_mut().unwrap().insert(key, value);
                    }
                    Ok(stats)
                })());
            },
        ))));
    }
}

struct ConnStateData {
    conn_uniq: Uniq,
    this: ConnStateWeak,
    metric_conn_count: UpDownObsAtomicI64,
    meta: ConnStateMeta,
    state: StateWeak,
    this_id: Id,
    rem_id: Id,
    conn_evt: ConnStateEvtSnd,
    sig_state: SigStateWeak,
    rcv_offer: bool,
    rcv_pending:
        HashMap<u64, (BytesList, Vec<tokio::sync::OwnedSemaphorePermit>)>,
    wait_preflight: bool,
    offer: (u64, u64, u64, u64),
    answer: (u64, u64, u64, u64),
    ice: (u64, u64, u64, u64),
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
            conn_uniq = %self.conn_uniq,
            this_id = ?self.this_id,
            rem_id = ?self.rem_id,
            "ConnShutdown",
        );
        self.metric_conn_count.add(-1);
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
            ConnCmd::Stats(rsp) => self.stats(rsp).await,
            ConnCmd::TrackSig { ty, bytes } => self.track_sig(ty, bytes).await,
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
            ConnCmd::Recv {
                ident,
                data,
                permit,
            } => self.recv(ident, data, permit).await,
        }
    }

    async fn tick_1s(&mut self) -> Result<()> {
        if self.meta.last_active_at.elapsed() > self.meta.config.max_conn_init()
            && !self.connected()
        {
            self.shutdown(Error::id("InactivityTimeout"));
        }

        Ok(())
    }

    async fn stats(
        &mut self,
        rsp: tokio::sync::oneshot::Sender<Result<serde_json::Value>>,
    ) -> Result<()> {
        let mut additions = Vec::new();
        additions.push((
            "ageSeconds".into(),
            self.meta.created_at.elapsed().as_secs_f64().into(),
        ));

        let sig_stats = serde_json::json!({
            "offersSent": self.offer.0,
            "offerBytesSent": self.offer.1,
            "offersReceived": self.offer.2,
            "offerBytesReceived": self.offer.3,
            "answersSent": self.answer.0,
            "answerBytesSent": self.answer.1,
            "answersReceived": self.answer.2,
            "answerBytesReceived": self.answer.3,
            "iceMessagesSent": self.ice.0,
            "iceBytesSent": self.ice.1,
            "iceMessagesReceived": self.ice.2,
            "iceBytesReceived": self.ice.3,
        });
        additions.push(("signalingTransport".into(), sig_stats));

        self.conn_evt.stats(additions, rsp);

        Ok(())
    }

    async fn track_sig(
        &mut self,
        ty: &'static str,
        bytes: usize,
    ) -> Result<()> {
        match ty {
            "offer_out" => {
                self.offer.0 += 1;
                self.offer.1 += bytes as u64;
            }
            "offer_in" => {
                self.offer.2 += 1;
                self.offer.3 += bytes as u64;
            }
            "answer_out" => {
                self.answer.0 += 1;
                self.answer.1 += bytes as u64;
            }
            "answer_in" => {
                self.answer.2 += 1;
                self.answer.3 += bytes as u64;
            }
            "ice_out" => {
                self.ice.0 += 1;
                self.ice.1 += bytes as u64;
            }
            "ice_in" => {
                self.ice.2 += 1;
                self.ice.3 += bytes as u64;
            }
            _ => (),
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

    async fn ice(&mut self, data: BackBuf) -> Result<()> {
        let sig = self.get_sig()?;
        sig.snd_ice(self.rem_id, data)
    }

    async fn self_offer(&mut self, offer: Result<BackBuf>) -> Result<()> {
        let sig = self.get_sig()?;
        let mut offer = offer?;
        self.conn_evt.set_loc(self.this.clone(), offer.try_clone()?);
        sig.snd_offer(self.rem_id, offer)
    }

    async fn req_self_answer(&mut self) -> Result<()> {
        self.conn_evt.create_answer(self.this.clone());
        Ok(())
    }

    async fn self_answer(&mut self, answer: Result<BackBuf>) -> Result<()> {
        let sig = self.get_sig()?;
        let mut answer = answer?;
        self.conn_evt
            .set_loc(self.this.clone(), answer.try_clone()?);
        sig.snd_answer(self.rem_id, answer)
    }

    async fn in_offer(&mut self, mut offer: BackBuf) -> Result<()> {
        tracing::trace!(
            conn_uniq = %self.conn_uniq,
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

    async fn in_answer(&mut self, mut answer: BackBuf) -> Result<()> {
        tracing::trace!(
            conn_uniq = %self.conn_uniq,
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

    async fn in_ice(&mut self, mut ice: BackBuf, cache: bool) -> Result<()> {
        tracing::trace!(
            conn_uniq = %self.conn_uniq,
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
        // first, check / send the preflight
        let data = self
            .meta
            .config
            .on_conn_preflight(self.meta.cli_url.clone())
            .await?
            .unwrap_or_else(bytes::Bytes::new);
        for buf in divide_send(&*self.meta.config, &self.meta.snd_ident, data)?
        {
            self.conn_evt.snd_data(self.this.clone(), buf, None, None);
        }

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
        if self.meta.created_at.elapsed()
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
        let SendData {
            msg_uniq,
            mut data,
            resp,
            send_permit,
            ..
        } = to_send;

        tracing::trace!(conn_uniq = %self.conn_uniq, %msg_uniq, "conn send");

        self.meta.last_active_at = std::time::Instant::now();
        self.meta.metric_bytes_snd.add(data.len()? as u64);

        self.conn_evt.snd_data(
            self.this.clone(),
            data,
            resp,
            Some(send_permit),
        );

        Ok(())
    }

    async fn handle_recv_data(
        &mut self,
        mut bl: BytesList,
        permit: Vec<tokio::sync::OwnedSemaphorePermit>,
    ) -> Result<()> {
        use bytes::Buf;

        if let Some(state) = self.state.upgrade() {
            if self.wait_preflight {
                let bytes = if bl.has_remaining() {
                    Some(bl.copy_to_bytes(bl.remaining()))
                } else {
                    None
                };
                self.meta
                    .config
                    .on_conn_validate(self.meta.cli_url.clone(), bytes)
                    .await?;
                self.wait_preflight = false;

                state.conn_ready(self.meta.cli_url.clone());
            } else {
                state.publish(StateEvt::RcvData(
                    self.meta.cli_url.clone(),
                    bl.into_dyn(),
                    vec![Permit(permit)],
                ));
            }
        }
        Ok(())
    }

    async fn recv(
        &mut self,
        ident: u64,
        data: bytes::Bytes,
        permit: tokio::sync::OwnedSemaphorePermit,
    ) -> Result<()> {
        let len = data.len();
        self.meta.last_active_at = std::time::Instant::now();

        self.meta.metric_bytes_rcv.add(len as u64);

        let is_finish = ident.is_finish();
        let ident = ident.unset_finish();

        match self.rcv_pending.entry(ident) {
            std::collections::hash_map::Entry::Vacant(e) => {
                if data.is_empty() || is_finish {
                    tracing::trace!(%is_finish, %ident, "rcv already finished");
                    // special case for oneshot message
                    let mut bl = BytesList::new();
                    if !data.is_empty() {
                        bl.push(data);
                    }
                    self.handle_recv_data(bl, vec![permit]).await?;
                } else {
                    tracing::trace!(%is_finish, %ident, byte_count=%len, "rcv new");
                    let mut bl = BytesList::new();
                    bl.push(data);
                    e.insert((bl, vec![permit]));
                }
            }
            std::collections::hash_map::Entry::Occupied(mut e) => {
                if data.is_empty() || is_finish {
                    tracing::trace!(%is_finish, %ident, "rcv complete");
                    // we've gotten to the end
                    let (mut bl, permit) = e.remove();
                    if !data.is_empty() {
                        bl.push(data);
                    }
                    self.handle_recv_data(bl, permit).await?;
                } else {
                    tracing::trace!(%is_finish, %ident, byte_count=%len, "rcv next");
                    e.get_mut().0.push(data);
                    e.get_mut().1.push(permit);
                }
            }
        }

        Ok(())
    }
}

enum ConnCmd {
    Tick1s,
    Stats(tokio::sync::oneshot::Sender<Result<serde_json::Value>>),
    TrackSig {
        ty: &'static str,
        bytes: usize,
    },
    NotifyConstructed,
    CheckConnectedTimeout,
    Ice {
        data: BackBuf,
    },
    SelfOffer {
        offer: Result<BackBuf>,
    },
    ReqSelfAnswer,
    SelfAnswer {
        answer: Result<BackBuf>,
    },
    InOffer {
        offer: BackBuf,
    },
    InAnswer {
        answer: BackBuf,
    },
    InIce {
        ice: BackBuf,
        cache: bool,
    },
    Ready,
    MaybeFetchForSend,
    Send {
        to_send: SendData,
    },
    Recv {
        ident: u64,
        data: bytes::Bytes,
        permit: tokio::sync::OwnedSemaphorePermit,
    },
}

#[allow(clippy::too_many_arguments)]
async fn conn_state_task(
    conn_limit: Arc<tokio::sync::Semaphore>,
    metric_conn_count: UpDownObsAtomicI64,
    meta: ConnStateMeta,
    strong: ConnState,
    conn_rcv: ManyRcv<ConnStateEvt>,
    mut rcv: ManyRcv<ConnCmd>,
    this: ConnStateWeak,
    state: StateWeak,
    conn_uniq: Uniq,
    this_id: Id,
    rem_id: Id,
    conn_evt: ConnStateEvtSnd,
    sig_state: SigStateWeak,
    sig_ready: tokio::sync::oneshot::Receiver<Result<Tx5Url>>,
) -> Result<()> {
    metric_conn_count.add(1);

    let mut data = ConnStateData {
        conn_uniq,
        this,
        metric_conn_count,
        meta,
        state,
        this_id,
        rem_id,
        conn_evt,
        sig_state,
        rcv_offer: false,
        rcv_pending: HashMap::new(),
        wait_preflight: true,
        offer: (0, 0, 0, 0),
        answer: (0, 0, 0, 0),
        ice: (0, 0, 0, 0),
    };

    let mut permit = None;

    let err = match async {
        if conn_limit.available_permits() < 1 {
            tracing::warn!(conn_uniq = %data.conn_uniq, "max connections reached, waiting for permit");
        }

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
                tracing::debug!(conn_uniq = %data.conn_uniq, id = ?data.rem_id, "NewConn");
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
    pub(crate) created_at: std::time::Instant,
    pub(crate) last_active_at: std::time::Instant,
    cli_url: Tx5Url,
    pub(crate) conn_uniq: Uniq,
    pub(crate) config: DynConfig,
    pub(crate) connected: Arc<atomic::AtomicBool>,
    _conn_snd: ConnStateEvtSnd,
    pub(crate) rcv_limit: Arc<tokio::sync::Semaphore>,
    pub(crate) metric_bytes_snd: CounterObsAtomicU64,
    pub(crate) metric_bytes_rcv: CounterObsAtomicU64,
    snd_ident: Arc<std::sync::atomic::AtomicU64>,
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

    /// Get the remote url of this connection.
    pub fn rem_url(&self) -> Tx5Url {
        self.1.cli_url.clone()
    }

    /// The connection generated an ice candidate for the remote.
    pub fn ice(&self, data: BackBuf) -> Result<()> {
        self.0.send(Ok(ConnCmd::Ice { data }))
    }

    /// The connection is ready to send and receive data.
    pub fn ready(&self) -> Result<()> {
        self.0.send(Ok(ConnCmd::Ready))
    }

    /// The connection received data on the data channel.
    /// This synchronous function must not block for now...
    /// (we'll need to test some blocking strategies
    /// for the goroutine in tx5-go-pion)... but we also can't just
    /// fill up memory if the application is processing slowly.
    /// So it will error / trigger connection shutdown if we get
    /// too much of a backlog.
    pub fn rcv_data(&self, mut data: BackBuf) -> Result<()> {
        // polling try_acquire doesn't fairly reserve a place in line,
        // so we need to timeout an actual acquire future..

        // we've got 15 ms of time to acquire the recv permit
        // this is a little more forgiving than just blanket deny
        // if the app is a little slow, but it's also not so much time
        // that we'll end up stalling other tasks that need to run.

        let fut = tokio::time::timeout(
            std::time::Duration::from_millis(15),
            async move {
                use std::io::Read;

                let mut len = data.len()?;
                if len > 16 * 1024 {
                    return Err(Error::id("MsgChunkTooLarge"));
                }
                if len < 8 {
                    return Err(Error::id("MsgChunkInvalid"));
                }

                let mut ident = [0; 8];
                data.read_exact(&mut ident[..])?;
                let ident = u64::from_le_bytes(ident);

                len -= 8;

                let buf = bytes::BytesMut::with_capacity(len);
                let mut buf = bytes::BufMut::writer(buf);
                std::io::copy(&mut data, &mut buf)?;
                let buf = buf.into_inner().freeze();

                if self.1.rcv_limit.available_permits() < len {
                    tracing::warn!(%len, "recv queue full, waiting for permits");
                }

                let permit = self
                    .1
                    .rcv_limit
                    .clone()
                    .acquire_many_owned(len as u32)
                    .await
                    .map_err(|_| Error::id("Closed"))?;

                self.0.send(Ok(ConnCmd::Recv {
                    ident,
                    data: buf,
                    permit,
                }))
            },
        );

        // need an external (futures) polling executor, because tokio
        // won't let us use blocking_recv, since we're still in the runtime.

        futures::executor::block_on(fut).map_err(|_| {
            tracing::error!("SLOW_APP: failed to receive in timely manner");
            Error::id("RecvQueueFull")
        })?
    }

    /// The send buffer *was* high, but has now transitioned to low.
    pub fn buf_amt_low(&self) -> Result<()> {
        todo!()
    }

    /// Get stats.
    pub async fn stats(&self) -> Result<serde_json::Value> {
        let (s, r) = tokio::sync::oneshot::channel();
        if self.0.send(Ok(ConnCmd::Stats(s))).is_err() {
            return Err(Error::id("Closed"));
        }
        r.await.map_err(|_| Error::id("Closed"))?
    }

    // -- //

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new_and_publish(
        config: DynConfig,
        conn_limit: Arc<tokio::sync::Semaphore>,
        metric_conn_count: UpDownObsAtomicI64,
        state: StateWeak,
        sig_state: SigStateWeak,
        state_uniq: Uniq,
        conn_uniq: Uniq,
        this_id: Id,
        cli_url: Tx5Url,
        rem_id: Id,
        rcv_limit: Arc<tokio::sync::Semaphore>,
        sig_ready: tokio::sync::oneshot::Receiver<Result<Tx5Url>>,
        maybe_offer: Option<BackBuf>,
        snd_ident: Arc<std::sync::atomic::AtomicU64>,
    ) -> Result<ConnStateWeak> {
        let (conn_snd, conn_rcv) = tokio::sync::mpsc::unbounded_channel();
        let conn_snd = ConnStateEvtSnd(conn_snd);

        let metric_bytes_snd = opentelemetry_api::global::meter_provider()
            .versioned_meter(
                "tx5",
                None::<&'static str>,
                None::<&'static str>,
                Some(vec![
                    opentelemetry_api::KeyValue::new(
                        "state_uniq",
                        state_uniq.to_string(),
                    ),
                    opentelemetry_api::KeyValue::new(
                        "conn_uniq",
                        conn_uniq.to_string(),
                    ),
                ]),
            )
            .u64_observable_counter_atomic("tx5.endpoint.conn.send", 0)
            .with_description("Outgoing bytes sent on this connection")
            .with_unit(opentelemetry_api::metrics::Unit::new("By"))
            .init()
            .0;

        let metric_bytes_rcv = opentelemetry_api::global::meter_provider()
            .versioned_meter(
                "tx5",
                None::<&'static str>,
                None::<&'static str>,
                Some(vec![
                    opentelemetry_api::KeyValue::new(
                        "state_uniq",
                        state_uniq.to_string(),
                    ),
                    opentelemetry_api::KeyValue::new(
                        "conn_uniq",
                        conn_uniq.to_string(),
                    ),
                ]),
            )
            .u64_observable_counter_atomic("tx5.endpoint.conn.recv", 0)
            .with_description("Incoming bytes received on this connection")
            .with_unit(opentelemetry_api::metrics::Unit::new("By"))
            .init()
            .0;

        let meta = ConnStateMeta {
            created_at: std::time::Instant::now(),
            last_active_at: std::time::Instant::now(),
            cli_url,
            conn_uniq: conn_uniq.clone(),
            config: config.clone(),
            connected: Arc::new(atomic::AtomicBool::new(false)),
            _conn_snd: conn_snd.clone(),
            rcv_limit,
            metric_bytes_snd,
            metric_bytes_rcv,
            snd_ident,
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
                    metric_conn_count,
                    meta.clone(),
                    strong,
                    ManyRcv(conn_rcv),
                    rcv,
                    ConnStateWeak(this, meta),
                    state,
                    conn_uniq,
                    this_id,
                    rem_id,
                    conn_snd,
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
            tokio::time::sleep(config.max_conn_init()).await;
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

    pub(crate) fn track_sig(&self, ty: &'static str, bytes: usize) {
        let _ = self.0.send(Ok(ConnCmd::TrackSig { ty, bytes }));
    }

    async fn check_connected_timeout(&self) {
        let _ = self.0.send(Ok(ConnCmd::CheckConnectedTimeout));
    }

    fn notify_constructed(&self) -> Result<()> {
        self.0.send(Ok(ConnCmd::NotifyConstructed))
    }

    fn self_offer(&self, offer: Result<BackBuf>) {
        let _ = self.0.send(Ok(ConnCmd::SelfOffer { offer }));
    }

    fn req_self_answer(&self) {
        let _ = self.0.send(Ok(ConnCmd::ReqSelfAnswer));
    }

    fn self_answer(&self, answer: Result<BackBuf>) {
        let _ = self.0.send(Ok(ConnCmd::SelfAnswer { answer }));
    }

    pub(crate) fn in_offer(&self, offer: BackBuf) {
        let _ = self.0.send(Ok(ConnCmd::InOffer { offer }));
    }

    pub(crate) fn in_answer(&self, answer: BackBuf) {
        let _ = self.0.send(Ok(ConnCmd::InAnswer { answer }));
    }

    pub(crate) fn in_ice(&self, mut ice: BackBuf, cache: bool) {
        let bytes = ice.len().unwrap();
        let _ = self.0.send(Ok(ConnCmd::TrackSig {
            ty: "ice_in",
            bytes,
        }));
        let _ = self.0.send(Ok(ConnCmd::InIce { ice, cache }));
    }

    pub(crate) async fn notify_send_waiting(&self) {
        let _ = self.0.send(Ok(ConnCmd::MaybeFetchForSend));
    }

    pub(crate) fn send(&self, to_send: SendData) {
        let _ = self.0.send(Ok(ConnCmd::Send { to_send }));
    }

    pub(crate) fn notify_send_complete(&self, _buffer_state: BufState) {
        // TODO - something with buffer state

        // for now just trigger a check for another message to send
        let _ = self.0.send(Ok(ConnCmd::MaybeFetchForSend));
    }
}
