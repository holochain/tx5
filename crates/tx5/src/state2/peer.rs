use super::*;

#[derive(PartialEq)]
pub(crate) enum PeerState {
    Connecting,
    #[allow(dead_code)]
    Open,
}

struct SendItem {
    _permit: tokio::sync::OwnedSemaphorePermit,
    #[allow(dead_code)]
    data: BackBuf,
    resp: Arc<Mutex<Option<tokio::sync::oneshot::Sender<Result<()>>>>>,
    msg_timeout_at: tokio::time::Instant,
}

pub(crate) struct Peer {
    pub peer_id: Id,
    pub peer_uniq: PeerUniq,
    #[allow(dead_code)]
    pub state: PeerState,
    #[allow(dead_code)]
    is_polite: bool,
    #[allow(dead_code)]
    is_outgoing: bool,
    #[allow(dead_code)]
    connect_timeout_at: tokio::time::Instant,
    send_queue: VecDeque<SendItem>,
}

impl Peer {
    pub fn new(
        assoc: &mut Assoc<'_, '_, '_>,
        sig_url: SigUrl,
        sig_uniq: SigUniq,
        peer_id: Id,
        is_polite: bool,
        is_outgoing: bool,
        ice_servers: Arc<serde_json::Value>,
    ) -> Self {
        let peer_uniq = uniq();

        tracing::info!(%sig_url, ?sig_uniq, ?peer_id, ?peer_uniq, "New Peer Connection");

        assoc.evt_list.push_back(State2Evt::PeerCreate {
            sig_url,
            sig_uniq,
            peer_id,
            peer_uniq,
            ice_servers,
        });

        Self {
            peer_id,
            peer_uniq,
            state: PeerState::Connecting,
            is_polite,
            is_outgoing,
            connect_timeout_at: tokio::time::Instant::now(),
            send_queue: VecDeque::new(),
        }
    }

    pub fn close(&mut self, assoc: &mut Assoc<'_, '_, '_>, err: Error) {
        tracing::info!(?self.peer_id, ?self.peer_uniq, ?err, "Peer Connection Close");

        assoc.evt_list.push_back(State2Evt::PeerClose {
            peer_id: self.peer_id,
            peer_uniq: self.peer_uniq,
            err,
        });
    }

    pub fn tick(&mut self, _assoc: &mut Assoc<'_, '_, '_>) -> Result<()> {
        // first timeout old messages
        let now = tokio::time::Instant::now();
        while {
            if let Some(msg) = self.send_queue.front() {
                now >= msg.msg_timeout_at
            } else {
                false
            }
        } {
            let SendItem { resp, .. } = self.send_queue.pop_front().unwrap();
            if let Some(s) = resp.lock().unwrap().take() {
                let _ = s.send(Err(Error::id("Timeout")));
            };
        }
        Ok(())
    }

    pub fn cmd(&mut self, _assoc: &mut Assoc<'_, '_, '_>, cmd: PeerCmd) {
        match cmd {}
    }

    pub fn send_msg(
        &mut self,
        assoc: &mut Assoc<'_, '_, '_>,
        permit: tokio::sync::OwnedSemaphorePermit,
        data: BackBuf,
        resp: Arc<Mutex<Option<tokio::sync::oneshot::Sender<Result<()>>>>>,
    ) {
        self.send_queue.push_back(SendItem {
            _permit: permit,
            data,
            resp,
            msg_timeout_at: tokio::time::Instant::now() + assoc.config.timeout,
        });
    }
}
