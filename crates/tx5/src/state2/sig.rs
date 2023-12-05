use super::*;

#[derive(Default)]
struct PeerMap(HashMap<Id, Peer>);

impl PeerMap {
    pub fn peer_assert(
        &mut self,
        assoc: &mut Assoc<'_, '_, '_>,
        peer_id: Id,
        is_polite: bool,
        is_outgoing: bool,
    ) -> PeerUniq {
        let r = self
            .0
            .entry(peer_id)
            .or_insert_with(|| Peer::new(assoc, peer_id, is_polite, is_outgoing));
        r.peer_uniq
    }

    pub fn peer_cmd(
        &mut self,
        assoc: &mut Assoc<'_, '_, '_>,
        peer_id: Id,
        peer_uniq: PeerUniq,
        peer_cmd: PeerCmd,
    ) {
        if let Some(peer) = self.0.get_mut(&peer_id) {
            if peer.peer_uniq != peer_uniq {
                return;
            }

            peer.cmd(assoc, peer_cmd);
        }
    }
}

#[derive(PartialEq)]
pub(crate) enum SigState {
    Disconnected,
    Connecting,
    Open,
}

pub(crate) struct Sig {
    pub sig_uniq: SigUniq,
    pub is_listening: bool,
    pub state: SigState,
    sig_url: SigUrl,
    backoff_interval: std::time::Duration,
    backoff_next_try_at: tokio::time::Instant,
    connect_timeout_at: tokio::time::Instant,
    peer_map: PeerMap,
}

impl Sig {
    pub fn new(assoc: &mut Assoc<'_, '_, '_>, sig_url: SigUrl) -> Self {
        let sig_uniq = uniq();

        *assoc.want_tick = true;

        let now = tokio::time::Instant::now();

        Self {
            sig_uniq,
            is_listening: false,
            state: SigState::Disconnected,
            sig_url,
            backoff_interval: assoc.config.backoff_interval_start,
            backoff_next_try_at: now,
            connect_timeout_at: now,
            peer_map: PeerMap::default(),
        }
    }

    pub fn close(&mut self, assoc: &mut Assoc<'_, '_, '_>, err: Error) {
        if self.state == SigState::Disconnected {
            return;
        }

        self.state = SigState::Disconnected;
        self.backoff_interval =
            self.backoff_interval.mul_f64(assoc.config.backoff_fact);
        if self.backoff_interval > assoc.config.backoff_interval_max {
            self.backoff_interval = assoc.config.backoff_interval_max;
        }

        self.backoff_next_try_at =
            tokio::time::Instant::now() + self.backoff_interval;

        assoc.evt_list.push_back(State2Evt::SigClose {
            sig_url: self.sig_url.clone(),
            sig_uniq: self.sig_uniq,
            err,
        });
    }

    pub fn tick(&mut self, assoc: &mut Assoc<'_, '_, '_>) -> Result<()> {
        let now = tokio::time::Instant::now();

        if self.state == SigState::Connecting && now >= self.connect_timeout_at
        {
            // don't call close directly, since we might need to
            // be removed from the map above us.
            return Err(Error::id("ConnectTimeout"));
        }

        if self.state == SigState::Disconnected
            && now >= self.backoff_next_try_at
        {
            self.sig_uniq = uniq();
            self.state = SigState::Connecting;
            assoc.evt_list.push_back(State2Evt::SigCreate {
                sig_url: self.sig_url.clone(),
                sig_uniq: self.sig_uniq,
            });
        }

        Ok(())
    }

    pub fn set_is_listening(&mut self) {
        self.is_listening = true;
    }

    pub fn cmd(&mut self, assoc: &mut Assoc<'_, '_, '_>, cmd: SigCmd) {
        match cmd {
            SigCmd::Open { id } => self.open(assoc, id),
            SigCmd::PeerAssert { peer_id, is_polite, is_outgoing } => {
                self.peer_assert(assoc, peer_id, is_polite, is_outgoing);
            }
            SigCmd::PeerCmd { peer_id, peer_uniq, peer_cmd } =>
                self.peer_map.peer_cmd(assoc, peer_id, peer_uniq, peer_cmd),
        }
    }

    pub fn open(&mut self, assoc: &mut Assoc<'_, '_, '_>, id: Id) {
        if assoc.this_id != &id {
            assoc.evt_list.push_back(State2Evt::SigClose {
                sig_url: self.sig_url.clone(),
                sig_uniq: self.sig_uniq,
                err: Error::id("InvalidSignalIdentity").into(),
            });
            return;
        }

        if self.state == SigState::Connecting {
            self.state = SigState::Open;
        }
    }

    pub fn peer_assert(
        &mut self,
        assoc: &mut Assoc<'_, '_, '_>,
        peer_id: Id,
        is_polite: bool,
        is_outgoing: bool,
    ) -> PeerUniq {
        self.peer_map.peer_assert(assoc, peer_id, is_polite, is_outgoing)
    }
}
