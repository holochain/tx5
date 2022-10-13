use super::*;

/// Temporary indicating we want a new conn instance.
pub struct ConnStateSeed {
    done: bool,
    output: Option<(ConnState, ManyRcv<Result<ConnStateEvt>>)>,
}

impl Drop for ConnStateSeed {
    fn drop(&mut self) {
        self.result_err_inner(Error::id("Dropped"));
    }
}

impl ConnStateSeed {
    /// Finalize this conn_state seed by indicating a successful connection.
    pub fn result_ok(
        mut self,
    ) -> Result<(ConnState, ManyRcv<Result<ConnStateEvt>>)> {
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
        conn_evt: ManyRcv<Result<ConnStateEvt>>,
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

#[derive(Clone)]
struct ConnStateEvtSnd(
    tokio::sync::mpsc::UnboundedSender<Result<ConnStateEvt>>,
);

impl ConnStateEvtSnd {
    pub fn err(&self, err: std::io::Error) {
        let _ = self.0.send(Err(err));
    }

    pub fn create_offer(&self) -> impl Future<Output = Result<Buf>> + 'static + Send {
        let (s, r) = tokio::sync::oneshot::channel();
        let s = OneSnd::new(move |result| {
            let _ = s.send(result);
        });
        let _ = self.0.send(Ok(ConnStateEvt::CreateOffer(s)));
        async move {
            r.await.map_err(|_| Error::id("Closed"))?
        }
    }
}

struct ConnStateData {
    this: ConnStateWeak,
    //state: StateWeak,
    rem_id: Id,
    conn_evt: ConnStateEvtSnd,
    sig_state: SigStateWeak,
    connected: bool,
}

impl Drop for ConnStateData {
    fn drop(&mut self) {
        self.shutdown(Error::id("Dropped"));
    }
}

impl ConnStateData {
    fn shutdown(&mut self, err: std::io::Error) {
        if true {
            todo!()
        }
        /*
        if let Some(state) = self.state.upgrade() {
            state.close_conn(self.rem_id, self.this.clone(), err.err_clone());
        }
        */
        if let Some(sig) = self.sig_state.upgrade() {
            sig.unregister_conn(self.rem_id, self.this.clone());
        }
        self.conn_evt.err(err);
    }

    fn get_sig(&mut self) -> Result<SigState> {
        match self.sig_state.upgrade() {
            Some(sig) => Ok(sig),
            None => Err(Error::id("SignalConnectionLost")),
        }
    }

    async fn exec(&mut self, cmd: ConnCmd) -> Result<()> {
        match cmd {
            ConnCmd::NotifyConstructed =>
                self.notify_constructed().await,
            ConnCmd::CheckConnectedTimeout =>
                self.check_connected_timeout().await,
            ConnCmd::Ice { data } =>
                self.ice(data).await,
        }
    }

    async fn notify_constructed(&mut self) -> Result<()> {
        todo!()
    }

    async fn check_connected_timeout(&mut self) -> Result<()> {
        if !self.connected {
            Err(Error::id("Timeout"))
        } else {
            Ok(())
        }
    }

    async fn ice(&mut self, data: Buf) -> Result<()> {
        match self.sig_state.upgrade() {
            None => Err(Error::id("SigClosed")),
            Some(sig) => {
                sig.snd_ice(self.rem_id, data)
            }
        }
    }
}

enum ConnCmd {
    NotifyConstructed,
    CheckConnectedTimeout,
    Ice { data: Buf },
}

async fn conn_state_task(
    mut rcv: ManyRcv<ConnCmd>,
    this: ConnStateWeak,
    // state: StateWeak,
    rem_id: Id,
    conn_evt: ConnStateEvtSnd,
    sig_state: SigStateWeak,
    sig_ready: tokio::sync::oneshot::Receiver<Result<()>>,
) -> Result<()> {
    let mut data = ConnStateData {
        this,
        //state,
        rem_id,
        conn_evt,
        sig_state,
        connected: false,
    };
    let err = match async {
        sig_ready.await.map_err(|_| Error::id("Closed"))??;
        match data.sig_state.upgrade() {
            None => return Err(Error::id("SigClosed")),
            Some(sig) =>
                sig.register_conn(data.rem_id, data.this.clone())?,
        }
        while let Some(cmd) = rcv.recv().await {
            data.exec(cmd?).await?;
        }
        Ok(())
    }.await {
        Err(err) => err,
        Ok(_) => Error::id("Dropped"),
    };
    data.shutdown(err.err_clone());
    Err(err)
}

/// Weak version on ConnState.
#[derive(Clone, PartialEq, Eq)]
pub struct ConnStateWeak(ActorWeak<ConnCmd>);

impl PartialEq<ConnState> for ConnStateWeak {
    fn eq(&self, rhs: &ConnState) -> bool {
        self.0 == rhs.0
    }
}

impl ConnStateWeak {
    /// Upgrade to a full ConnState instance.
    pub fn upgrade(&self) -> Option<ConnState> {
        self.0.upgrade().map(ConnState)
    }
}

/// A handle for notifying the state system of connection events.
#[derive(Clone, PartialEq, Eq)]
pub struct ConnState(Actor<ConnCmd>);

impl PartialEq<ConnStateWeak> for ConnState {
    fn eq(&self, rhs: &ConnStateWeak) -> bool {
        self.0 == rhs.0
    }
}

impl ConnState {
    /// Get a weak version of this ConnState instance.
    pub fn weak(&self) -> ConnStateWeak {
        ConnStateWeak(self.0.weak())
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

    /// The connection received data on the data channel.
    /// This synchronous function must not block for now...
    /// (we'll need to test some blocking strategies
    /// for the goroutine in tx4-go-pion)... but we also can't just
    /// fill up memory if the application is processing slowly.
    /// So it will error / trigger connection shutdown if we get
    /// too much of a backlog.
    pub fn rcv_data(&self, _data: Buf) -> Result<()> {
        // not super atomic, but a best effort : )
        if self.0.is_closed() {
            return Err(Error::id("Closed"));
        }
        todo!()
    }

    /// The send buffer *was* high, but has now transitioned to low.
    pub fn buf_amt_low(&self) -> Result<()> {
        // not super atomic, but a best effort : )
        if self.0.is_closed() {
            return Err(Error::id("Closed"));
        }
        todo!()
    }

    // -- //

    pub(crate) fn new(
        //state: StateWeak,
        sig_state: SigStateWeak,
        rem_id: Id,
        sig_ready: tokio::sync::oneshot::Receiver<Result<()>>,
    ) -> (Self, ManyRcv<ConnStateEvt>) {
        let (conn_snd, conn_rcv) = tokio::sync::mpsc::unbounded_channel();
        let actor = Actor::new(move |this, rcv| conn_state_task(
            rcv,
            ConnStateWeak(this),
            // state,
            rem_id,
            ConnStateEvtSnd(conn_snd),
            sig_state,
            sig_ready,
        ));
        let weak = ConnStateWeak(actor.weak());
        tokio::task::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(20)).await;
            if let Some(actor) = weak.upgrade() {
                actor.check_connected_timeout().await;
            }
        });
        (Self(actor), ManyRcv(conn_rcv))
    }

    async fn check_connected_timeout(&self) {
        let _ = self.0.send(Ok(ConnCmd::CheckConnectedTimeout));
    }

    fn notify_constructed(&self) -> Result<()> {
        self.0.send(Ok(ConnCmd::NotifyConstructed))
        /*
        let offer_fut = self.0
            .exec(move |inner| async move {
                let fut = inner.conn_evt.create_offer();
                (Some(inner), Ok(fut))
            })
            .await?;
        let this = self.clone();
        println!("GotOFFER");
        tokio::task::spawn(async move {
            let _offer = match offer_fut.await {
                Err(err) => {
                    this.close(err);
                    return;
                }
                Ok(offer) => offer,
            };
            let _ = this.0.exec(move |mut inner| async move {
                if let Err(err) = {
                    let inner = &mut inner;
                    async move {
                        let _sig = inner.get_sig()?;
                        //sig.send_offer(offer).await?;
                        if true {
                            todo!()
                        }
                        Ok(())
                    }.await
                } {
                    inner.shutdown(err);
                    return (None, Ok(()));
                }
                (Some(inner), Ok(()))
            }).await;
            println!("SENT OFFERR");
        });
        Ok(())
        */
    }

    pub(crate) async fn notify_send_waiting(&self) {
        todo!()
    }
}
