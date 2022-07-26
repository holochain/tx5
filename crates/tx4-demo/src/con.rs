use crate::*;
use std::sync::atomic;

pub struct Con {
    con_send: ConSend,
}

impl Drop for Con {
    fn drop(&mut self) {
        let _ = self.con_send.send(ConCmd::Shutdown);
    }
}

impl Con {
    pub fn new(
        core: core::Core,
        state: state::State,
        id: state::PeerId,
        ice_servers: serde_json::Value,
        is_out: bool,
    ) -> Self {
        let (con_send, con_recv) = tokio::sync::mpsc::unbounded_channel();

        debug_spawn(con_task(
            core,
            state,
            id,
            ice_servers,
            is_out,
            con_send.clone(),
            con_recv,
        ));

        Self { con_send }
    }

    pub fn offer(&self, offer: serde_json::Value) {
        let _ = self.con_send.send(ConCmd::Offer(offer));
    }

    pub fn answer(&self, answer: serde_json::Value) {
        let _ = self.con_send.send(ConCmd::Answer(answer));
    }

    pub fn ice(&self, ice: serde_json::Value) {
        let _ = self.con_send.send(ConCmd::Ice(ice));
    }
}

enum ConCmd {
    Shutdown,
    DataChannel(tx4_go_pion::DataChannelSeed),
    Offer(serde_json::Value),
    Answer(serde_json::Value),
    Ice(serde_json::Value),
    DataChanOpen,
    DataChanClose,
    DataChanMsg(tx4_go_pion::GoBuf),
}

type ConSend = tokio::sync::mpsc::UnboundedSender<ConCmd>;
type ConRecv = tokio::sync::mpsc::UnboundedReceiver<ConCmd>;

async fn con_task(
    core: core::Core,
    state: state::State,
    id: state::PeerId,
    ice_servers: serde_json::Value,
    is_out: bool,
    con_send: ConSend,
    mut con_recv: ConRecv,
) -> Result<()> {
    tracing::debug!(?id, ?is_out, "open con");

    let should_block = Arc::new(atomic::AtomicBool::new(true));

    struct DoneDrop {
        core: core::Core,
        id: state::PeerId,
        should_block: Arc<atomic::AtomicBool>,
    }

    impl Drop for DoneDrop {
        fn drop(&mut self) {
            tracing::debug!(id = ?self.id, "CON DROP");
            let should_block = self.should_block.load(atomic::Ordering::Relaxed);
            self.core.drop_con(self.id.clone(), should_block);
        }
    }

    let _done_drop = DoneDrop {
        core: core.clone(),
        id: id.clone(),
        should_block: should_block.clone(),
    };

    let conf = serde_json::to_string(&serde_json::json!({
        "iceServers": ice_servers,
    }))?;

    let handle_data_chan = {
        let con_send = con_send.clone();
        move |seed: tx4_go_pion::DataChannelSeed| {
            let con_send2 = con_send.clone();
            let mut ch = seed.handle(move |evt| match evt {
                tx4_go_pion::DataChannelEvent::Open => {
                    let _ = con_send2.send(ConCmd::DataChanOpen);
                }
                tx4_go_pion::DataChannelEvent::Close => {
                    let _ = con_send2.send(ConCmd::DataChanClose);
                }
                tx4_go_pion::DataChannelEvent::Message(msg) => {
                    let _ = con_send2.send(ConCmd::DataChanMsg(msg));
                }
            });
            if ch.ready_state().map_err(other_err)? == 2 {
                let _ = con_send.send(ConCmd::DataChanOpen);
            }
            Result::Ok(ch)
        }
    };

    let mut peer_con = {
        let core = core.clone();
        let id = id.clone();
        tx4_go_pion::PeerConnection::new(&conf, move |evt| match evt {
            tx4_go_pion::PeerConnectionEvent::ICECandidate(ice) => {
                core.send_ice(id.clone(), ice);
            }
            tx4_go_pion::PeerConnectionEvent::DataChannel(data_chan) => {
                let _ = con_send.send(ConCmd::DataChannel(data_chan));
            }
        })
        .map_err(other_err)?
    };

    let mut data_chan = None;

    let mut msg_sent = false;
    let mut msg_recvd = false;

    if is_out {
        let ch = peer_con
            .create_data_channel("{ \"label\": \"data\" }")
            .map_err(other_err)?;
        data_chan = Some(handle_data_chan(ch)?);
        let offer = peer_con.create_offer(None).map_err(other_err)?;
        peer_con.set_local_description(&offer).map_err(other_err)?;
        core.send_offer(id.clone(), offer);
    }

    while let Some(cmd) = con_recv.recv().await {
        match cmd {
            ConCmd::Shutdown => break,
            ConCmd::DataChannel(ch) => {
                if data_chan.is_none() {
                    data_chan = Some(handle_data_chan(ch)?);
                    tracing::trace!("got data channel");
                }
            }
            ConCmd::Offer(offer) => {
                let offer = serde_json::to_string(&offer)?;
                peer_con.set_remote_description(&offer).map_err(other_err)?;
                let answer = peer_con.create_answer(None).map_err(other_err)?;
                peer_con.set_local_description(&answer).map_err(other_err)?;
                tracing::trace!(%offer, %answer, "recv offer, gen answer");
                core.send_answer(id.clone(), answer);
            }
            ConCmd::Answer(answer) => {
                let answer = serde_json::to_string(&answer)?;
                peer_con
                    .set_remote_description(&answer)
                    .map_err(other_err)?;
                tracing::trace!(%answer, "recv answer");
            }
            ConCmd::Ice(ice) => {
                let ice = serde_json::to_string(&ice)?;
                tracing::trace!(%ice, "recv ice");
                if let Err(err) = peer_con.add_ice_candidate(&ice) {
                    tracing::trace!(?err, %ice, "invalid ice (can happen with perfect negotiation)");
                }
            }
            ConCmd::DataChanOpen => {
                tracing::trace!("data chan OPEN!");
                let msg = state.gen_msg()?;
                data_chan.as_mut().unwrap().send(msg).map_err(other_err)?;
                if msg_recvd {
                    break;
                }
                msg_sent = true;
            }
            ConCmd::DataChanClose => {
                return Err(other_err("DataChannelClosed"));
            }
            ConCmd::DataChanMsg(mut msg) => {
                use std::io::Read;
                tracing::trace!("DATA CHANNEL MESSAGE!!!");
                let mut buf = [0; 32];
                msg.read_exact(&mut buf[..])?;
                let len = buf.iter().position(|&c| c == b'\0').unwrap_or(buf.len());
                let friendly_name = String::from_utf8_lossy(&buf[..len]).to_string();
                msg.read_exact(&mut buf[..])?;
                let len = buf.iter().position(|&c| c == b'\0').unwrap_or(buf.len());
                let shoutout = String::from_utf8_lossy(&buf[..len]).to_string();

                let mut buf2 = [0; 32];
                loop {
                    if msg.read_exact(&mut buf[..]).is_err() {
                        break;
                    }

                    if msg.read_exact(&mut buf2[..]).is_err() {
                        break;
                    }

                    let rem_id = match tx4_signal_core::Id::from_slice(&buf) {
                        Err(_) => break,
                        Ok(id) => id,
                    };

                    let rem_pk = match tx4_signal_core::Id::from_slice(&buf2) {
                        Err(_) => break,
                        Ok(id) => id,
                    };

                    state.discover(rem_id, rem_pk);
                }

                state.contact(id.clone(), friendly_name, shoutout);

                if msg_sent {
                    break;
                }

                msg_recvd = true;
            }
        }
    }

    // we made it without erroring, say we should NOT block
    should_block.store(false, atomic::Ordering::Relaxed);

    Ok(())
}
