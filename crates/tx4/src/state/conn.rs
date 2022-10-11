use super::*;

/// Temporary indicating we want a new conn instance.
pub struct ConnStateSeed {
    done: bool,
    conn: ConnStateWeak,
    //conn_evt: Option<ManyRcv<Result<ConnStateEvt>>>,
}

impl Drop for ConnStateSeed {
    fn drop(&mut self) {
        self.result_err_inner(Error::id("Dropped"));
    }
}

impl ConnStateSeed {
    /// Finalize this sig_state seed by indicating a successful sig connection.
    pub fn result_ok(mut self) -> (ConnState, ManyRcv<Result<ConnStateEvt>>) {
        self.done = true;
        /*
        self.state_data.new_conn_ok(
            self.key,
            self.rem_id.clone(),
            self.sig_url.clone(),
        );
        let conn_state = ConnState {};
        let conn_evt = self.conn_evt.take().unwrap();
        (conn_state, conn_evt)
        */
        todo!()
    }

    /// Finalize this sig_state seed by indicating an error connecting.
    pub fn result_err(mut self, err: std::io::Error) {
        self.result_err_inner(err);
    }

    fn result_err_inner(&mut self, err: std::io::Error) {
        if !self.done {
            self.done = true;

            if let Some(conn) = self.conn.upgrade() {
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
}

struct ConnStateData {
    this: ConnStateWeak,
    state: StateWeak,
    rem_id: Id,
    conn_evt: ConnStateEvtSnd,
    sig_state: SigStateWeak,
}

impl Drop for ConnStateData {
    fn drop(&mut self) {
        self.shutdown(Error::id("Dropped"));
    }
}

impl ConnStateData {
    fn shutdown(&mut self, err: std::io::Error) {
        if let Some(state) = self.state.upgrade() {
            state.close_conn(self.this.clone(), err.err_clone());
        }
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
}

/// Weak version on ConnState.
#[derive(Clone, PartialEq, Eq)]
pub struct ConnStateWeak(ActorWeak<ConnStateData>);

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
pub struct ConnState(Actor<ConnStateData>);

impl PartialEq<ConnStateWeak> for ConnState {
    fn eq(&self, rhs: &ConnStateWeak) -> bool {
        self.0 == rhs.0
    }
}

impl ConnState {
    /// Returns `true` if this ConnState is closed.
    pub fn is_closed(&self) -> bool {
        self.0.is_closed()
    }

    /// Shutdown the connection with an error.
    pub fn close(&self, err: std::io::Error) {
        tokio::task::spawn(self.0.exec_close(|mut inner| async move {
            inner.shutdown(err);
            (None, Ok(()))
        }));
    }

    /// The connection generated an ice candidate for the remote.
    pub fn ice(&self, data: Buf) -> Result<()> {
        // not super atomic, but a best effort : )
        if self.0.is_closed() {
            return Err(Error::id("Closed"));
        }
        tokio::task::spawn(self.0.exec(move |mut inner| async move {
            if let Err(err) = {
                let inner = &mut inner;
                (move || {
                    let sig = inner.get_sig()?;
                    sig.ice(inner.rem_id, data)?;
                    Ok(())
                })()
            } {
                inner.shutdown(err);
                return (None, Ok(()));
            }
            (Some(inner), Ok(()))
        }));
        Ok(())
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
}

/*
/// A handle for notifying the state system of connection events.
pub struct ConnState {}

impl ConnState {
    /// Shutdown the connection with an optional error.
    pub fn shutdown(&self, _err: std::io::Error) {
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
