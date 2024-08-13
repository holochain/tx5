use crate::{AbortTask, CloseRecv, CloseSend};
use std::io::{Error, Result};

pub enum WebrtcEvt {
    GeneratedOffer(Vec<u8>),
    GeneratedAnswer(Vec<u8>),
    GeneratedIce(Vec<u8>),
    Message(Vec<u8>),
    Ready,
}

enum Cmd {
    InOffer(Vec<u8>),
    InAnswer(Vec<u8>),
    InIce(Vec<u8>),
    GeneratedIce(Vec<u8>),
    DataChan(
        tx5_go_pion::DataChannel,
        tokio::sync::mpsc::UnboundedReceiver<tx5_go_pion::DataChannelEvent>,
    ),
    SendMessage(Vec<u8>, tokio::sync::oneshot::Sender<()>),
    RecvMessage(Vec<u8>),
    DataChanOpen,
    BufferedAmountLow,
}

pub struct Webrtc {
    cmd_send: CloseSend<Cmd>,
    _task: AbortTask<Result<()>>,
    _evt_send: CloseSend<WebrtcEvt>,
}

impl Webrtc {
    pub fn new(
        is_polite: bool,
        config: Vec<u8>,
        send_buffer: usize,
    ) -> (Self, CloseRecv<WebrtcEvt>) {
        let (mut cmd_send, cmd_recv) = CloseSend::channel();
        let (mut evt_send, evt_recv) = CloseSend::channel();

        let task = tokio::task::spawn(task(
            is_polite,
            config,
            send_buffer,
            evt_send.clone(),
            cmd_send.clone(),
            cmd_recv,
        ));

        cmd_send.set_close_on_drop(true);
        evt_send.set_close_on_drop(true);

        (
            Self {
                cmd_send,
                _task: AbortTask(task),
                _evt_send: evt_send,
            },
            evt_recv,
        )
    }

    pub async fn in_offer(&self, offer: Vec<u8>) -> Result<()> {
        self.cmd_send
            .send(Cmd::InOffer(offer))
            .await
            .map_err(|_| Error::other("closed"))
    }

    pub async fn in_answer(&self, answer: Vec<u8>) -> Result<()> {
        self.cmd_send
            .send(Cmd::InAnswer(answer))
            .await
            .map_err(|_| Error::other("closed"))
    }

    pub async fn in_ice(&self, ice: Vec<u8>) -> Result<()> {
        self.cmd_send
            .send(Cmd::InIce(ice))
            .await
            .map_err(|_| Error::other("closed"))
    }

    pub async fn message(&self, message: Vec<u8>) -> Result<()> {
        let (s, r) = tokio::sync::oneshot::channel();
        self.cmd_send
            .send(Cmd::SendMessage(message, s))
            .await
            .map_err(|_| Error::other("closed"))?;
        let _ = r.await;
        Ok(())
    }
}

async fn task(
    is_polite: bool,
    config: Vec<u8>,
    send_buffer: usize,
    mut evt_send: CloseSend<WebrtcEvt>,
    cmd_send: CloseSend<Cmd>,
    mut cmd_recv: CloseRecv<Cmd>,
) -> Result<()> {
    evt_send.set_close_on_drop(true);

    let (peer, mut peer_evt) = tx5_go_pion::PeerConnection::new(config).await?;

    let mut cmd_send2 = cmd_send.clone();
    let _peer_task: AbortTask<Result<()>> =
        AbortTask(tokio::task::spawn(async move {
            cmd_send2.set_close_on_drop(true);

            use tx5_go_pion::PeerConnectionEvent as Evt;
            while let Some(evt) = peer_evt.recv().await {
                match evt {
                    Evt::Error(_) => break,
                    Evt::State(_) => (),
                    Evt::ICECandidate(mut ice) => {
                        cmd_send2
                            .send(Cmd::GeneratedIce(ice.to_vec()?))
                            .await?;
                    }
                    Evt::DataChannel(d, dr) => {
                        cmd_send2.send(Cmd::DataChan(d, dr)).await?;
                    }
                }
            }
            Ok(())
        }));

    let mut offer = None;
    let mut data = None;
    let mut _data_recv = None;
    let mut did_handshake = false;
    let mut pend_buffer = Vec::new();

    if !is_polite {
        let (d, dr) = peer
            .create_data_channel(b"{\"label\":\"data\"}".to_vec())
            .await?;
        d.set_buffered_amount_low_threshold(send_buffer)?;
        data = Some(d);
        _data_recv = spawn_data_chan(cmd_send.clone(), dr);
        let mut o = peer.create_offer(b"{}".to_vec()).await?;
        evt_send
            .send(WebrtcEvt::GeneratedOffer(o.to_vec()?))
            .await?;
        offer = Some(o);
    }

    loop {
        let cmd = match cmd_recv.recv().await {
            None => break,
            Some(cmd) => cmd,
        };

        let mut slow_task = "unknown";

        match breakable_timeout!(match cmd {
            Cmd::InOffer(o) => {
                slow_task = "in-offer";
                if is_polite && !did_handshake {
                    peer.set_remote_description(o).await?;
                    let mut a = peer.create_answer(b"{}".to_vec()).await?;
                    evt_send
                        .send(WebrtcEvt::GeneratedAnswer(a.to_vec()?))
                        .await?;
                    peer.set_local_description(a).await?;
                    did_handshake = true;
                }
            }
            Cmd::InAnswer(a) => {
                slow_task = "in-answer";
                if !is_polite && !did_handshake {
                    if let Some(o) = offer.take() {
                        peer.set_local_description(o).await?;
                        peer.set_remote_description(a).await?;
                        did_handshake = true;
                    }
                }
            }
            Cmd::InIce(i) => {
                slow_task = "in-ice";
                let _ = peer.add_ice_candidate(i).await;
            }
            Cmd::GeneratedIce(ice) => {
                slow_task = "gen-ice";
                evt_send.send(WebrtcEvt::GeneratedIce(ice)).await?;
            }
            Cmd::DataChan(d, dr) => {
                slow_task = "data-chan";
                if data.is_none() {
                    d.set_buffered_amount_low_threshold(send_buffer)?;
                    data = Some(d);
                    _data_recv = spawn_data_chan(cmd_send.clone(), dr);
                }
            }
            Cmd::SendMessage(msg, resp) => {
                slow_task = "send-msg";
                if let Some(d) = &data {
                    let amt = match d.send(msg).await {
                        Ok(amt) => amt,
                        Err(_) => break,
                    };
                    if amt <= send_buffer {
                        drop(resp);
                        pend_buffer.clear();
                    } else {
                        pend_buffer.push(resp);
                    }
                } else {
                    break;
                }
            }
            Cmd::RecvMessage(msg) => {
                slow_task = "recv-msg";
                evt_send.send(WebrtcEvt::Message(msg)).await?;
            }
            Cmd::DataChanOpen => {
                slow_task = "chan-open";
                evt_send.send(WebrtcEvt::Ready).await?;
            }
            Cmd::BufferedAmountLow => {
                slow_task = "buf-low";
                pend_buffer.clear();
            }
        }) {
            Err(_) => {
                let err = format!("slow app on webrtc loop task: {slow_task}");
                tracing::warn!("{err}");
                Err(Error::other(err))
            }
            Ok(r) => r,
        }?;
    }

    Ok(())
}

fn spawn_data_chan(
    mut cmd_send: CloseSend<Cmd>,
    mut data_recv: tokio::sync::mpsc::UnboundedReceiver<
        tx5_go_pion::DataChannelEvent,
    >,
) -> Option<AbortTask<Result<()>>> {
    use tx5_go_pion::DataChannelEvent as Evt;
    Some(AbortTask(tokio::task::spawn(async move {
        cmd_send.set_close_on_drop(true);

        // Receiving on the unbounded data channel receiver has a real
        // chance to fill our memory with message data.
        // We give a small chance for the app to catch up, otherwise
        // error so the connection will close.
        while let Some(evt) = data_recv.recv().await {
            match evt {
                Evt::Error(_) => break,
                Evt::Open => {
                    cmd_send.send_slow_app(Cmd::DataChanOpen).await?;
                }
                Evt::Close => break,
                Evt::Message(mut msg) => {
                    cmd_send
                        .send_slow_app(Cmd::RecvMessage(msg.to_vec()?))
                        .await?;
                }
                Evt::BufferedAmountLow => {
                    cmd_send.send_slow_app(Cmd::BufferedAmountLow).await?;
                }
            }
        }
        Ok(())
    })))
}
