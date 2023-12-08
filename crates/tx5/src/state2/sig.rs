use super::*;

#[derive(Default)]
struct PeerMap(HashMap<Id, Peer>);

impl PeerMap {
    pub fn peer_assert(
        &mut self,
        assoc: &mut Assoc<'_, '_, '_>,
        sig_url: SigUrl,
        sig_uniq: SigUniq,
        peer_id: Id,
        is_polite: bool,
        is_outgoing: bool,
        ice_servers: Arc<serde_json::Value>,
    ) -> &mut Peer {
        let r = self.0.entry(peer_id).or_insert_with(|| {
            Peer::new(assoc, sig_url, sig_uniq, peer_id, is_polite, is_outgoing, ice_servers)
        });
        r
    }

    pub fn peer_close(
        &mut self,
        assoc: &mut Assoc<'_, '_, '_>,
        peer_id: Id,
        peer_uniq: PeerUniq,
        err: Error,
    ) {
        if let Some(mut peer) = self.0.remove(&peer_id) {
            if peer.peer_uniq != peer_uniq {
                // woops!
                self.0.insert(peer_id, peer);
                return;
            }

            peer.close(assoc, err);
        }
    }

    pub fn tick(&mut self, assoc: &mut Assoc<'_, '_, '_>) {
        let mut rm = Vec::new();

        for (peer_id, peer) in self.0.iter_mut() {
            if let Err(err) = peer.tick(assoc) {
                rm.push((*peer_id, peer.peer_uniq, err));
            }
        }

        for (peer_id, peer_uniq, err) in rm {
            self.peer_close(assoc, peer_id, peer_uniq, err.into());
        }
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
        tracing::info!(?self.sig_uniq, %self.sig_url, "Signal Connection Close");
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
            self.connect_timeout_at = now + assoc.config.timeout;

            tracing::info!(?self.sig_uniq, %self.sig_url, "New Signal Connection");

            assoc.evt_list.push_back(State2Evt::SigCreate {
                sig_url: self.sig_url.clone(),
                sig_uniq: self.sig_uniq,
            });
        }

        self.peer_map.tick(assoc);

        Ok(())
    }

    pub fn set_is_listening(&mut self) {
        self.is_listening = true;
    }

    pub fn cmd(&mut self, assoc: &mut Assoc<'_, '_, '_>, cmd: SigCmd) {
        match cmd {
            SigCmd::Open {
                this_id,
                ice_servers,
            } => self.open(assoc, this_id, ice_servers),
            SigCmd::PeerAssert {
                peer_id,
                is_polite,
                is_outgoing,
            } => {
                self.peer_assert(assoc, peer_id, is_polite, is_outgoing);
            }
            SigCmd::PeerClose {
                peer_id,
                peer_uniq,
                err,
            } => self.peer_map.peer_close(assoc, peer_id, peer_uniq, err),
            SigCmd::PeerCmd {
                peer_id,
                peer_uniq,
                peer_cmd,
            } => self.peer_map.peer_cmd(assoc, peer_id, peer_uniq, peer_cmd),
        }
    }

    pub fn open(
        &mut self,
        assoc: &mut Assoc<'_, '_, '_>,
        this_id: Id,
        _ice_servers: Arc<serde_json::Value>,
    ) {
        if assoc.this_id != &this_id {
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
        ice_
    ) -> &mut Peer {
        self.peer_map.peer_assert(
            assoc,
            self.sig_url.clone(),
            self.sig_uniq,
            peer_id,
            is_polite,
            is_outgoing,
        )
    }
}
