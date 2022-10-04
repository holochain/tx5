use super::*;

/// Temporary indicating we want a new signal instance.
pub struct SigStateSeed {
    pub(crate) done: bool,
    pub(crate) key: u64,
    pub(crate) url: Arc<str>,
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
    pub fn result_ok(mut self) -> (SigState, ManyRcv<Result<SigStateEvt>>) {
        self.done = true;
        self.state_data.new_listener_sig_ok(self.key, &self.url);
        let sig_state = SigState(self.state_data.clone());
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
            .new_listener_sig_err(self.key, &self.url, err);
    }
}

/// State wishes to invoke an action on a signal instance.
pub enum SigStateEvt {
    /// Forward an offer to a remote.
    SndOffer(Id, Buf, OneSnd<Result<()>>),

    /// Forward an answer to a remote.
    SndAnswer(Id, Buf, OneSnd<Result<()>>),

    /// Forward an ICE candidate to a remote
    SndIce(Id, Buf, OneSnd<Result<()>>),
}

/// A handle for notifying the state system of signal events.
pub struct SigState(StateData);

impl SigState {
    /// Shutdown the signal client with an optional error.
    pub fn shutdown(&self, _maybe_err: Option<Error>) {
        todo!()
    }

    /// Receive an incoming offer from a remote.
    pub fn offer(&self, _id: Id, _data: Buf) -> Result<()> {
        todo!()
    }

    /// Receive an incoming answer from a remote.
    pub fn answer(&self, _id: Id, _data: Buf) -> Result<()> {
        todo!()
    }

    /// Receive an incoming ice candidate from a remote.
    pub fn ice(&self, _id: Id, _data: Buf) -> Result<()> {
        todo!()
    }
}
