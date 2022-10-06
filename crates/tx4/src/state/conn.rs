use super::*;

/// Temporary indicating we want a new conn instance.
pub struct ConnStateSeed {
    pub(crate) done: bool,
    pub(crate) key: Key,
    pub(crate) rem_id: Id,
    pub(crate) sig_url: Tx4Url,
    pub(crate) state_data: StateData,
    pub(crate) conn_evt: Option<ManyRcv<Result<ConnStateEvt>>>,
}

impl Drop for ConnStateSeed {
    fn drop(&mut self) {
        if !self.done {
            self.result_err_inner(Error::id("Dropped"));
        }
    }
}

impl ConnStateSeed {
    /// Finalize this sig_state seed by indicating a successful sig connection.
    pub fn result_ok(
        mut self,
    ) -> (ConnState, ManyRcv<Result<ConnStateEvt>>) {
        self.done = true;
        self.state_data
            .new_conn_ok(self.key, self.rem_id.clone(), self.sig_url.clone());
        let conn_state = ConnState {
        };
        let conn_evt = self.conn_evt.take().unwrap();
        (conn_state, conn_evt)
    }

    /// Finalize this sig_state seed by indicating an error connecting.
    pub fn result_err(mut self, err: std::io::Error) {
        self.result_err_inner(err);
    }

    fn result_err_inner(&mut self, err: std::io::Error) {
        self.done = true;
        self.state_data
            .new_conn_err(self.key, self.rem_id.clone(), err);
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
