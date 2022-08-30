use crate::*;
use std::collections::HashMap;

pub struct Core {
    is_main: bool,
    core_send: CoreSend,
}

impl Clone for Core {
    fn clone(&self) -> Self {
        Self {
            is_main: false,
            core_send: self.core_send.clone(),
        }
    }
}

impl Drop for Core {
    fn drop(&mut self) {
        if self.is_main {
            let _ = self.core_send.send(CoreCmd::Shutdown);
        }
    }
}

impl Core {
    pub fn new(friendly_name: String, shoutout: String) -> Self {
        let (core_send, core_recv) = tokio::sync::mpsc::unbounded_channel();

        let main_core = Self {
            is_main: true,
            core_send: core_send.clone(),
        };

        tokio::task::spawn(core_task(
            friendly_name,
            shoutout,
            main_core.clone(),
            core_send,
            core_recv,
        ));

        main_core
    }

    pub fn addr(&self, addr: url::Url) {
        let _ = self.core_send.send(CoreCmd::Addr(addr));
    }

    pub fn sig(&self, sig: sig::Sig) {
        let _ = self.core_send.send(CoreCmd::Sig(sig));
    }

    pub fn ice(&self, ice: serde_json::Value) {
        let _ = self.core_send.send(CoreCmd::Ice(ice));
    }

    pub fn sig_msg(&self, msg: tx4_signal::cli::SigMessage) {
        let _ = self.core_send.send(CoreCmd::SigMsg(msg));
    }

    pub fn drop_con(&self, id: state::PeerId, should_block: bool) {
        let _ = self.core_send.send(CoreCmd::DropCon(id, should_block));
    }

    pub fn send_offer(&self, id: state::PeerId, offer: String) {
        let _ = self.core_send.send(CoreCmd::SendOffer(id, offer));
    }

    pub fn send_answer(&self, id: state::PeerId, answer: String) {
        let _ = self.core_send.send(CoreCmd::SendAnswer(id, answer));
    }

    pub fn send_ice(&self, id: state::PeerId, ice: String) {
        let _ = self.core_send.send(CoreCmd::SendIce(id, ice));
    }
}

enum CoreCmd {
    Shutdown,
    Tick,
    Addr(url::Url),
    Ice(serde_json::Value),
    Sig(sig::Sig),
    SigMsg(tx4_signal::cli::SigMessage),
    DropCon(state::PeerId, bool),
    SendOffer(state::PeerId, String),
    SendAnswer(state::PeerId, String),
    SendIce(state::PeerId, String),
}

type CoreSend = tokio::sync::mpsc::UnboundedSender<CoreCmd>;
type CoreRecv = tokio::sync::mpsc::UnboundedReceiver<CoreCmd>;

#[allow(unused_variables, unused_assignments)]
async fn core_task(
    friendly_name: String,
    shoutout: String,
    core: Core,
    core_send: CoreSend,
    mut core_recv: CoreRecv,
) {
    let tick_abort = tokio::task::spawn(async move {
        loop {
            use rand::Rng;
            let secs = rand::thread_rng().gen_range(3..8);
            tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
            if core_send.send(CoreCmd::Tick).is_err() {
                break;
            }
        }
    });

    struct KillTick(tokio::task::JoinHandle<()>);

    impl Drop for KillTick {
        fn drop(&mut self) {
            tracing::warn!("DemoShuttingDown");
            self.0.abort();
        }
    }

    let _kill_tick = KillTick(tick_abort);

    use tx4_signal::cli::SigMessage;

    let state = state::State::new(friendly_name, shoutout);

    let mut ice_servers = serde_json::json!([]);
    let mut sig = None;
    let mut con_map = HashMap::new();
    let mut loc_pk = tx4_signal::Id::from_slice(&[0; 32]).unwrap();

    while let Some(cmd) = core_recv.recv().await {
        match cmd {
            CoreCmd::Shutdown => break,
            CoreCmd::Tick => {
                tracing::trace!(?state, "tick");
                for id in state.check_want_outgoing() {
                    // this is an "outgoing" connection,
                    // that is, the one that will make the webrtc "offer".
                    let is_out = true;
                    let con = con_::Con::new(
                        core.clone(),
                        state.clone(),
                        id.clone(),
                        ice_servers.clone(),
                        is_out,
                    );
                    con_map.insert(id, con);
                }
            }
            CoreCmd::DropCon(id, should_block) => {
                con_map.remove(&id);
                state.con_done(id, should_block);
            }
            CoreCmd::Addr(addr) => {
                let loc_id = tx4_signal::signal_id_from_addr(&addr).unwrap();
                loc_pk = tx4_signal::pk_from_addr(&addr).unwrap();
                tracing::info!(?loc_id, ?loc_pk, "recv local id");
                state.set_loc(loc_id, loc_pk.clone());
            }
            CoreCmd::Ice(got_ice) => {
                ice_servers = got_ice;
                tracing::info!(%ice_servers);
            }
            CoreCmd::Sig(got_sig) => {
                sig = Some(got_sig);
            }
            CoreCmd::SigMsg(msg) => match msg {
                SigMessage::Offer {
                    rem_id,
                    rem_pk,
                    offer,
                } => {
                    // *this* node will be polite if our loc_pk < rem_pk,
                    // (collisions are UB), see:
                    // https://developer.mozilla.org/en-US/docs/Web/API/WebRTC_API/Perfect_negotiation
                    let loc_polite = loc_pk.0 < rem_pk.0;

                    let id = state::PeerId { rem_id, rem_pk };

                    tracing::trace!(?loc_polite, "recv offer");

                    if con_map.contains_key(&id) {
                        if loc_polite {
                            // we are the *polite* node, drop
                            // our previous offer
                            let should_block = false;
                            state.con_done(id.clone(), should_block);
                            con_map.remove(&id);

                            tracing::trace!(?loc_polite, "drop our offer");
                        } else {
                            // we are the *impolite* node, ignore the
                            // remote offer
                            tracing::trace!(?loc_polite, "IgnoreOffer");
                            continue;
                        }
                    }

                    if state.check_want_incoming(id.clone()) {
                        // this is an "incoming" connection,
                        // that is, the one that accepts the webrtc "offer",
                        // then creates the webrtc "answer".
                        let is_out = false;
                        let con = con_::Con::new(
                            core.clone(),
                            state.clone(),
                            id.clone(),
                            ice_servers.clone(),
                            is_out,
                        );
                        con.offer(offer);
                        con_map.insert(id, con);
                    }
                }
                SigMessage::Answer {
                    rem_id,
                    rem_pk,
                    answer,
                } => {
                    let id = state::PeerId { rem_id, rem_pk };
                    if let Some(con) = con_map.get(&id) {
                        con.answer(answer);
                    }
                }
                SigMessage::ICE {
                    rem_id,
                    rem_pk,
                    ice,
                } => {
                    let id = state::PeerId { rem_id, rem_pk };
                    if let Some(con) = con_map.get(&id) {
                        con.ice(ice);
                    }
                }
                SigMessage::Demo { rem_id, rem_pk } => {
                    state.discover(rem_id, rem_pk);
                }
            },
            CoreCmd::SendOffer(id, offer) => {
                sig.as_ref().unwrap().send_offer(id, offer);
            }
            CoreCmd::SendAnswer(id, answer) => {
                sig.as_ref().unwrap().send_answer(id, answer);
            }
            CoreCmd::SendIce(id, ice) => {
                sig.as_ref().unwrap().send_ice(id, ice);
            }
        }
    }
}
