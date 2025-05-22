use super::*;
use crate::{AbortTask, CloseRecv, CloseSend};
use datachannel::TransportPolicy;
use std::io::{Error, Result};

type MapErr<E, F> = Box<dyn FnOnce(E) -> F>;
fn map_err<E: std::fmt::Debug>(s: &'static str) -> MapErr<E, std::io::Error> {
    Box::new(move |e| std::io::Error::other(format!("{s}: {e:?}")))
}

enum Cmd {
    InOffer(Vec<u8>),
    InAnswer(Vec<u8>),
    InIce(Vec<u8>),
    GeneratedIce(datachannel::IceCandidate),
    DataChan(Box<datachannel::RtcDataChannel<Dch>>),
    SendMessage(Vec<u8>, tokio::sync::oneshot::Sender<()>),
    RecvMessage(Vec<u8>),
    RecvDescription(Box<datachannel::SessionDescription>),
    DataChanOpen,
    BufferedAmountLow,
    Error(std::io::Error),
}

/// Data channel handler.
struct Dch(CloseSend<Cmd>);

impl datachannel::DataChannelHandler for Dch {
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

/// Peer connection handler.
struct Pch(CloseSend<Cmd>);

impl datachannel::PeerConnectionHandler for Pch {
    type DCH = Dch;

    fn data_channel_handler(
        &mut self,
        _info: datachannel::DataChannelInfo,
    ) -> Self::DCH {
        Dch(self.0.clone())
    }

    fn on_description(&mut self, sess_desc: datachannel::SessionDescription) {
        let _ = self
            .0
            .send_or_close(Cmd::RecvDescription(Box::new(sess_desc)));
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

/// Libdatachannel backed webrtc implementation.
pub struct Webrtc {
    cmd_send: CloseSend<Cmd>,
    _task: AbortTask<()>,
    _evt_send: CloseSend<WebrtcEvt>,
}

impl Webrtc {
    /// Construct a new libdatachannel backed webrtc implementation.
    #[allow(clippy::new_ret_no_self)]
    #[allow(clippy::needless_return)]
    pub fn new(
        is_polite: bool,
        config: WebRtcConfig,
        send_buffer: usize,
    ) -> (DynWebrtc, CloseRecv<WebrtcEvt>) {
        // make some of the library calls faster by preloading once
        static PRELOAD: std::sync::Once = std::sync::Once::new();
        PRELOAD.call_once(datachannel::preload);

        // initialize tracing once
        static INIT_TRACING: std::sync::Once = std::sync::Once::new();
        INIT_TRACING.call_once(|| {
            use tracing::event_enabled;
            use tracing::Level;

            if !tx5_core::Tx5InitConfig::get().tracing_enabled {
                return;
            }

            if event_enabled!(target: "datachannel", Level::TRACE) {
                datachannel::configure_logging(Level::TRACE);
                return;
            }
            if event_enabled!(target: "datachannel", Level::DEBUG) {
                datachannel::configure_logging(Level::DEBUG);
                return;
            }
            if event_enabled!(target: "datachannel", Level::INFO) {
                datachannel::configure_logging(Level::INFO);
                return;
            }
            if event_enabled!(target: "datachannel", Level::WARN) {
                datachannel::configure_logging(Level::WARN);
                return;
            }
            if event_enabled!(target: "datachannel", Level::ERROR) {
                datachannel::configure_logging(Level::ERROR);
                return;
            }
        });

        let (mut cmd_send, cmd_recv) = CloseSend::sized_channel(1024);
        let (mut evt_send, evt_recv) = CloseSend::sized_channel(1024);

        // spawn the background task
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
        // forward the offer to our task
        Box::pin(async move {
            self.cmd_send
                .send_or_close(Cmd::InOffer(offer))
                .map_err(|_| Error::other("closed"))
        })
    }

    fn in_answer(&self, answer: Vec<u8>) -> BoxFuture<'_, Result<()>> {
        // forward the answer to our task
        Box::pin(async move {
            self.cmd_send
                .send_or_close(Cmd::InAnswer(answer))
                .map_err(|_| Error::other("closed"))
        })
    }

    fn in_ice(&self, ice: Vec<u8>) -> BoxFuture<'_, Result<()>> {
        // forward the ice candidate to our task
        Box::pin(async move {
            self.cmd_send
                .send_or_close(Cmd::InIce(ice))
                .map_err(|_| Error::other("closed"))
        })
    }

    fn message(&self, message: Vec<u8>) -> BoxFuture<'_, Result<()>> {
        // forward the message to our task and await it actually being sent
        Box::pin(async move {
            tracing::trace!(byte_len = message.len(), "datachannel queue send");
            let (s, r) = tokio::sync::oneshot::channel();
            self.cmd_send
                .send_or_close(Cmd::SendMessage(message, s))
                .map_err(|_| Error::other("closed"))?;
            let _ = r.await;
            Ok(())
        })
    }
}

async fn task(
    is_polite: bool,
    config: WebRtcConfig,
    send_buffer: usize,
    evt_send: CloseSend<WebrtcEvt>,
    cmd_send: CloseSend<Cmd>,
    cmd_recv: CloseRecv<Cmd>,
) {
    if let Err(err) =
        task_err(is_polite, config, send_buffer, evt_send, cmd_send, cmd_recv)
            .await
    {
        tracing::warn!(?err, "webrtc task error");
    }
}

async fn task_err(
    is_polite: bool,
    config: WebRtcConfig,
    send_buffer: usize,
    mut evt_send: CloseSend<WebrtcEvt>,
    cmd_send: CloseSend<Cmd>,
    mut cmd_recv: CloseRecv<Cmd>,
) -> Result<()> {
    evt_send.set_close_on_drop(true);

    let init_config = tx5_core::Tx5InitConfig::get();

    // set up the webrtc config
    // TODO - max_message_size?
    let config = datachannel::RtcConfig::new::<String>(
        &config
            .ice_servers
            .iter()
            .flat_map(|s| s.urls.iter())
            .cloned()
            .collect::<Vec<_>>(),
    )
    .ice_transport_policy(match config.ice_transport_policy {
        crate::config::TransportPolicy::All => TransportPolicy::All,
        crate::config::TransportPolicy::Relay => TransportPolicy::Relay,
    })
    .port_range_begin(init_config.ephemeral_udp_port_min)
    .port_range_end(init_config.ephemeral_udp_port_max);

    // create the actual peer connection, passing in our handler
    let mut peer =
        datachannel::RtcPeerConnection::new(&config, Pch(cmd_send.clone()))
            .map_err(map_err("constructing peer connection"))?;

    let mut data = None;
    let mut did_handshake = false;
    let mut pend_buffer = Vec::new();

    // if we are the impolite node, create the datachannel and be the offerer
    if !is_polite {
        // create a datachannel, passing in our handler
        let mut d = peer
            .create_data_channel("data", Dch(cmd_send.clone()))
            .map_err(map_err("creating data channel"))?;
        d.set_buffered_amount_low_threshold(send_buffer)
            .map_err(map_err("setting buffer low threshold (out)"))?;
        data = Some(d);
        peer.set_local_description(datachannel::SdpType::Offer)
            .map_err(map_err("setting local desc to offer"))?;
    }

    // loop on incoming commands
    loop {
        let cmd = match cmd_recv.recv().await {
            None => break,
            Some(cmd) => cmd,
        };

        match cmd {
            Cmd::InOffer(o) => {
                // only polite nodes should receive offers
                if is_polite && !did_handshake {
                    let o: datachannel::SessionDescription =
                        serde_json::from_slice(&o)
                            .map_err(map_err("deserializing remote offer"))?;
                    peer.set_remote_description(&o)
                        .map_err(map_err("setting remote offer desc"))?;
                    // NOTE: I guess this auto-answers??
                    // We get a Runtime error if we call this explicitly:
                    //peer.set_local_description(datachannel::SdpType::Answer)
                    //    .map_err(map_err("setting local desc to answer"))?;
                    did_handshake = true;
                }
            }
            Cmd::InAnswer(a) => {
                // only impolite nodes should receive answers
                if !is_polite && !did_handshake {
                    let a: datachannel::SessionDescription =
                        serde_json::from_slice(&a)
                            .map_err(map_err("deserializing remote answer"))?;
                    peer.set_remote_description(&a)
                        .map_err(map_err("setting remote answer desc"))?;
                    did_handshake = true;
                }
            }
            Cmd::InIce(i) => {
                // all nodes receive trickle ice candidates from the remote peer
                let i: datachannel::IceCandidate =
                    serde_json::from_slice(&i)
                        .map_err(map_err("deserializing remote candidate"))?;
                if let Err(err) = peer
                    .add_remote_candidate(&i)
                    .map_err(map_err("adding remote candidate"))
                {
                    // Don't error on ice candidates, it might be from
                    // a previous negotiation, just note it in the trace
                    tracing::warn!(?err, "failed to add remote candidate");
                }
            }
            Cmd::GeneratedIce(ice) => {
                // as libdatachannel generates candidates,
                // forward them to our remote peer
                evt_send.send_or_close(WebrtcEvt::GeneratedIce(
                    serde_json::to_string(&ice)?.into_bytes(),
                ))?;
            }
            Cmd::DataChan(mut d) => {
                // the polite peer should receive the data channel
                // that was created by the impolite one.
                if data.is_none() {
                    d.set_buffered_amount_low_threshold(send_buffer).map_err(
                        map_err("setting buffer low threshold (in)"),
                    )?;
                    data = Some(d);
                } else {
                    // we cannot proceed if a datachannel was created in error
                    return Err(std::io::Error::other("duplicate data chan"));
                }
            }
            Cmd::SendMessage(msg, resp) => {
                // send an outgoing message over the data channel
                if let Some(d) = &mut data {
                    d.send(&msg).map_err(map_err("sending message"))?;
                    let amt = d.buffered_amount();
                    tracing::trace!(
                        byte_len = msg.len(),
                        buffer_amt = amt,
                        "datachannel sent"
                    );

                    // Depending on how much is buffered, either drop the resp
                    // (i.e. allow new messages to be sent), or queue it up
                    // to be dropped when we get a BufferedAmountLow event.
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
                // If we receive a message, send it to the evt receiver.
                evt_send.send_or_close(WebrtcEvt::Message(msg))?;
            }
            Cmd::RecvDescription(desc) => match desc.sdp_type {
                datachannel::SdpType::Offer => {
                    // send the offer to our remote peer
                    evt_send.send_or_close(WebrtcEvt::GeneratedOffer(
                        serde_json::to_string(&desc)?.into_bytes(),
                    ))?;
                }
                datachannel::SdpType::Answer => {
                    // send the answer to our remote peer
                    evt_send.send_or_close(WebrtcEvt::GeneratedAnswer(
                        serde_json::to_string(&desc)?.into_bytes(),
                    ))?;
                }
                _ => {
                    return Err(std::io::Error::other(
                        "unhandled sdp desc type",
                    ))
                }
            },
            Cmd::DataChanOpen => {
                // Mark the instance as ready to send/receive
                evt_send.send_or_close(WebrtcEvt::Ready)?;
            }
            Cmd::BufferedAmountLow => {
                // notify any pending sends that their data has been sent
                // (or at least handed off to the backend)
                pend_buffer.clear();
            }
            Cmd::Error(err) => return Err(err),
        }
    }

    Ok(())
}
