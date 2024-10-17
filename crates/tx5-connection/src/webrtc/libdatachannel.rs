use super::*;
use crate::{AbortTask, CloseRecv, CloseSend};
use std::io::{Error, Result};

enum Cmd {
    InOffer(Vec<u8>),
    InAnswer(Vec<u8>),
    InIce(Vec<u8>),
    GeneratedIce(datachannel::IceCandidate),
    DataChan(Box<datachannel::RtcDataChannel<DCH>>),
    SendMessage(Vec<u8>, tokio::sync::oneshot::Sender<()>),
    RecvMessage(Vec<u8>),
    RecvDescription(datachannel::SessionDescription),
    DataChanOpen,
    BufferedAmountLow,
    Error(std::io::Error),
}

struct DCH(CloseSend<Cmd>);

impl datachannel::DataChannelHandler for DCH {
    fn on_open(&mut self) {
        let _ = self.0.send_or_close(Cmd::DataChanOpen);
    }

    fn on_closed(&mut self) {
        let _ = self
            .0
            .send_or_close(Cmd::Error(std::io::Error::other("DataChanClosed")));
    }

    fn on_error(&mut self, err: &str) {
        let _ =
            self.0
                .send_or_close(Cmd::Error(std::io::Error::other(format!(
                    "DataChanError: {err}"
                ))));
    }

    fn on_message(&mut self, msg: &[u8]) {
        let _ = self.0.send_or_close(Cmd::RecvMessage(msg.to_vec()));
    }

    fn on_buffered_amount_low(&mut self) {
        let _ = self.0.send_or_close(Cmd::BufferedAmountLow);
    }

    /*
    fn on_available(&mut self) {
        // TODO - figure out what this is
    }
    */
}

struct PCH(CloseSend<Cmd>);

impl datachannel::PeerConnectionHandler for PCH {
    type DCH = DCH;

    fn data_channel_handler(
        &mut self,
        _info: datachannel::DataChannelInfo,
    ) -> Self::DCH {
        DCH(self.0.clone())
    }

    fn on_description(&mut self, sess_desc: datachannel::SessionDescription) {
        let _ = self.0.send_or_close(Cmd::RecvDescription(sess_desc));
    }

    fn on_candidate(&mut self, cand: datachannel::IceCandidate) {
        let _ = self.0.send_or_close(Cmd::GeneratedIce(cand));
    }

    fn on_data_channel(
        &mut self,
        data_channel: Box<datachannel::RtcDataChannel<Self::DCH>>,
    ) {
        let _ = self.0.send_or_close(Cmd::DataChan(data_channel));
    }
}

pub struct Webrtc {
    cmd_send: CloseSend<Cmd>,
    _task: AbortTask<()>,
    _evt_send: CloseSend<WebrtcEvt>,
}

impl Webrtc {
    pub fn new(
        is_polite: bool,
        config: Vec<u8>,
        send_buffer: usize,
    ) -> (DynWebrtc, CloseRecv<WebrtcEvt>) {
        let (mut cmd_send, cmd_recv) = CloseSend::sized_channel(1024);
        let (mut evt_send, evt_recv) = CloseSend::sized_channel(1024);

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

        let this: DynWebrtc = Arc::new(Self {
            cmd_send,
            _task: AbortTask(task),
            _evt_send: evt_send,
        });

        (this, evt_recv)
    }
}

impl super::Webrtc for Webrtc {
    fn in_offer(&self, offer: Vec<u8>) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            self.cmd_send
                .send(Cmd::InOffer(offer))
                .await
                .map_err(|_| Error::other("closed"))
        })
    }

    fn in_answer(&self, answer: Vec<u8>) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            self.cmd_send
                .send(Cmd::InAnswer(answer))
                .await
                .map_err(|_| Error::other("closed"))
        })
    }

    fn in_ice(&self, ice: Vec<u8>) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            self.cmd_send
                .send(Cmd::InIce(ice))
                .await
                .map_err(|_| Error::other("closed"))
        })
    }

    fn message(&self, message: Vec<u8>) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            let (s, r) = tokio::sync::oneshot::channel();
            self.cmd_send
                .send(Cmd::SendMessage(message, s))
                .await
                .map_err(|_| Error::other("closed"))?;
            let _ = r.await;
            Ok(())
        })
    }
}

async fn task(
    is_polite: bool,
    config: Vec<u8>,
    send_buffer: usize,
    evt_send: CloseSend<WebrtcEvt>,
    cmd_send: CloseSend<Cmd>,
    cmd_recv: CloseRecv<Cmd>,
) {
    if let Err(err) = task_err(is_polite, config, send_buffer, evt_send, cmd_send, cmd_recv).await {
        tracing::debug!(?err, "webrtc task error");
    }
}

async fn task_err(
    is_polite: bool,
    _config: Vec<u8>,
    send_buffer: usize,
    mut evt_send: CloseSend<WebrtcEvt>,
    cmd_send: CloseSend<Cmd>,
    mut cmd_recv: CloseRecv<Cmd>,
) -> Result<()> {
    evt_send.set_close_on_drop(true);

    // TODO - use actual ice servers
    // TODO - max_message_size
    // TODO - port range begin/end
    let config = datachannel::RtcConfig::new::<String>(&[]);
    let mut peer =
        datachannel::RtcPeerConnection::new(&config, PCH(cmd_send.clone()))
            .map_err(std::io::Error::other)?;

    let mut data = None;
    let mut did_handshake = false;
    let mut pend_buffer = Vec::new();

    if !is_polite {
        let mut d = peer
            .create_data_channel("data", DCH(cmd_send.clone()))
            .map_err(std::io::Error::other)?;
        d.set_buffered_amount_low_threshold(send_buffer)
            .map_err(std::io::Error::other)?;
        data = Some(d);
        peer.set_local_description(datachannel::SdpType::Offer)
            .map_err(std::io::Error::other)?;
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
                    let o: datachannel::SessionDescription =
                        serde_json::from_slice(&o)
                            .map_err(std::io::Error::other)?;
                    peer.set_remote_description(&o)
                        .map_err(std::io::Error::other)?;
                    peer.set_local_description(datachannel::SdpType::Answer)
                        .map_err(std::io::Error::other)?;
                    did_handshake = true;
                }
            }
            Cmd::InAnswer(a) => {
                slow_task = "in-answer";
                if !is_polite && !did_handshake {
                    let a: datachannel::SessionDescription =
                        serde_json::from_slice(&a)
                            .map_err(std::io::Error::other)?;
                    peer.set_remote_description(&a)
                        .map_err(std::io::Error::other)?;
                    did_handshake = true;
                }
            }
            Cmd::InIce(i) => {
                slow_task = "in-ice";
                let i: datachannel::IceCandidate =
                    serde_json::from_slice(&i)
                        .map_err(std::io::Error::other)?;
                let _ = peer
                    .add_remote_candidate(&i)
                    .map_err(std::io::Error::other)?;
            }
            Cmd::GeneratedIce(ice) => {
                slow_task = "gen-ice";
                evt_send
                    .send(WebrtcEvt::GeneratedIce(
                        serde_json::to_string(&ice)?.into_bytes(),
                    ))
                    .await?;
            }
            Cmd::DataChan(mut d) => {
                slow_task = "data-chan";
                if data.is_none() {
                    d.set_buffered_amount_low_threshold(send_buffer)
                        .map_err(std::io::Error::other)?;
                    data = Some(d);
                } else {
                    return Err(std::io::Error::other("duplicate data chan"));
                }
            }
            Cmd::SendMessage(msg, resp) => {
                slow_task = "send-msg";
                if let Some(d) = &mut data {
                    d.send(&msg).map_err(std::io::Error::other)?;
                    let amt = d.buffered_amount();
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
            Cmd::RecvDescription(desc) => {
                slow_task = "recv-desc";
                match desc.sdp_type {
                    datachannel::SdpType::Offer => {
                        evt_send
                            .send(WebrtcEvt::GeneratedOffer(
                                serde_json::to_string(&desc)?.into_bytes(),
                            ))
                            .await?;
                    }
                    datachannel::SdpType::Answer => {
                        evt_send
                            .send(WebrtcEvt::GeneratedAnswer(
                                serde_json::to_string(&desc)?.into_bytes(),
                            ))
                            .await?;
                    }
                    _ => {
                        return Err(std::io::Error::other(
                            "unhandled sdp desc type",
                        ))
                    }
                }
            }
            Cmd::DataChanOpen => {
                slow_task = "chan-open";
                evt_send.send(WebrtcEvt::Ready).await?;
            }
            Cmd::BufferedAmountLow => {
                slow_task = "buf-low";
                pend_buffer.clear();
            }
            Cmd::Error(err) => return Err(err),
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
