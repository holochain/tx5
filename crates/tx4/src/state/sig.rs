use super::*;

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
        self.state_data
            .new_listener_sig_ok(self.key, self.sig_url.clone(), cli_url);
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
        self.state_data
            .new_listener_sig_err(self.key, self.sig_url.clone(), err);
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
        self.inner.new_listener_sig_err(self.key, self.sig_url.clone(), err);
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
        self.inner.check_incoming_answer(self.sig_url.clone(), rem_id, data)
    }

    /// Receive an incoming ice candidate from a remote.
    pub fn ice(&self, _rem_id: Id, _data: Buf) -> Result<()> {
        todo!()
    }
}
