//! Tx5 endpoint.

use crate::*;
use tx5_core::Tx5Url;

/// Event type emitted by a tx5 endpoint.
pub enum EpEvt {
    /// Connection established.
    Connected {
        /// The remote client url connected.
        rem_cli_url: Tx5Url,
    },

    /// Connection closed.
    Disconnected {
        /// The remote client url disconnected.
        rem_cli_url: Tx5Url,
    },

    /// Received data from a remote.
    Data {
        /// The remote client url that sent this message.
        rem_cli_url: Tx5Url,

        /// The payload of the message.
        data: Box<dyn bytes::Buf + 'static + Send>,

        /// Drop this when you've accepted the data to allow additional
        /// incoming messages.
        permit: Vec<state::Permit>,
    },

    /// Received a demo broadcast.
    Demo {
        /// The remote client url that is available for communication.
        rem_cli_url: Tx5Url,
    },
}

impl std::fmt::Debug for EpEvt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EpEvt::Connected { rem_cli_url } => f
                .debug_struct("EpEvt::Connected")
                .field("rem_cli_url", rem_cli_url)
                .finish(),
            EpEvt::Disconnected { rem_cli_url } => f
                .debug_struct("EpEvt::Disconnected")
                .field("rem_cli_url", rem_cli_url)
                .finish(),
            EpEvt::Data {
                rem_cli_url,
                data,
                permit: _,
            } => {
                let data_len = data.remaining();
                f.debug_struct("EpEvt::Data")
                    .field("rem_cli_url", rem_cli_url)
                    .field("data_len", &data_len)
                    .finish()
            }
            EpEvt::Demo { rem_cli_url } => f
                .debug_struct("EpEvt::Demo")
                .field("rem_cli_url", rem_cli_url)
                .finish(),
        }
    }
}

/// A tx5 endpoint representing an instance that can send and receive.
#[derive(Clone, PartialEq, Eq)]
pub struct Ep {
    state: state::State,
}

impl std::fmt::Debug for Ep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ep").finish()
    }
}

impl Ep {
    /// Construct a new tx5 endpoint.
    pub async fn new() -> Result<(Self, actor::ManyRcv<EpEvt>)> {
        Self::with_config(DefConfig::default()).await
    }

    /// Construct a new tx5 endpoint with configuration.
    pub async fn with_config<I: IntoConfig>(
        into_config: I,
    ) -> Result<(Self, actor::ManyRcv<EpEvt>)> {
        let (ep_snd, ep_rcv) = tokio::sync::mpsc::unbounded_channel();

        let config = into_config.into_config().await?;
        let (state, mut state_evt) = state::State::new(config.clone())?;
        tokio::task::spawn(async move {
            while let Some(evt) = state_evt.recv().await {
                match evt {
                    Ok(state::StateEvt::NewSig(sig_url, seed)) => {
                        config.on_new_sig(sig_url, seed);
                    }
                    Ok(state::StateEvt::Address(_cli_url)) => {}
                    Ok(state::StateEvt::NewConn(ice_servers, seed)) => {
                        config.on_new_conn(ice_servers, seed);
                    }
                    Ok(state::StateEvt::RcvData(url, buf, permit)) => {
                        let _ = ep_snd.send(Ok(EpEvt::Data {
                            rem_cli_url: url,
                            data: buf,
                            permit,
                        }));
                    }
                    Ok(state::StateEvt::Demo(cli_url)) => {
                        let _ = ep_snd.send(Ok(EpEvt::Demo {
                            rem_cli_url: cli_url,
                        }));
                    }
                    Ok(state::StateEvt::Connected(cli_url)) => {
                        let _ = ep_snd.send(Ok(EpEvt::Connected {
                            rem_cli_url: cli_url,
                        }));
                    }
                    Ok(state::StateEvt::Disconnected(cli_url)) => {
                        let _ = ep_snd.send(Ok(EpEvt::Disconnected {
                            rem_cli_url: cli_url,
                        }));
                    }
                    Err(err) => {
                        let _ = ep_snd.send(Err(err));
                        break;
                    }
                }
            }
        });
        let ep = Self { state };
        Ok((ep, actor::ManyRcv(ep_rcv)))
    }

    /// Establish a listening connection to a signal server,
    /// from which we can accept incoming remote connections.
    /// Returns the client url at which this endpoint may now be addressed.
    pub fn listen(
        &self,
        sig_url: Tx5Url,
    ) -> impl std::future::Future<Output = Result<Tx5Url>> + 'static + Send
    {
        self.state.listener_sig(sig_url)
    }

    /// Send data to a remote on this tx5 endpoint.
    pub fn send<B: bytes::Buf>(
        &self,
        rem_cli_url: Tx5Url,
        data: B,
    ) -> impl std::future::Future<Output = Result<()>> + 'static + Send {
        self.state.snd_data(rem_cli_url, data)
    }

    /// Send a demo broadcast to every connected signal server.
    /// Warning, if demo mode is not enabled on these servers, this
    /// could result in a ban.
    pub fn demo(&self) -> Result<()> {
        self.state.snd_demo()
    }
}

pub(crate) fn on_new_sig(
    config: DynConfig,
    sig_url: Tx5Url,
    seed: state::SigStateSeed,
) {
    tokio::task::spawn(new_sig_task(config, sig_url, seed));
}

async fn new_sig_task(
    config: DynConfig,
    sig_url: Tx5Url,
    seed: state::SigStateSeed,
) {
    tracing::debug!(%sig_url, "spawning new signal task");

    let (sig_snd, mut sig_rcv) = tokio::sync::mpsc::unbounded_channel();

    let (sig, cli_url) = match async {
        let sig = tx5_signal::Cli::builder()
            .with_lair_client(config.lair_client().clone())
            .with_lair_tag(config.lair_tag().clone())
            .with_url(sig_url.to_string().parse().unwrap())
            .with_recv_cb(move |msg| {
                let _ = sig_snd.send(msg);
            })
            .build()
            .await?;

        let cli_url = Tx5Url::new(sig.local_addr())?;

        Result::Ok((sig, cli_url))
    }
    .await
    {
        Ok(r) => r,
        Err(err) => {
            tracing::error!(?err, "error connecting to signal server");
            seed.result_err(err);
            return;
        }
    };

    tracing::debug!(%cli_url, "signal connection established");

    let sig = &sig;

    let ice_servers = sig.ice_servers().clone();

    let (sig_state, mut sig_evt) = match seed.result_ok(cli_url, ice_servers) {
        Err(_) => return,
        Ok(r) => r,
    };

    loop {
        tokio::select! {
            msg = sig_rcv.recv() => {
                if let Err(err) = async {
                    match msg {
                        Some(tx5_signal::SignalMsg::Demo { rem_pub }) => {
                            sig_state.demo(rem_pub)
                        }
                        Some(tx5_signal::SignalMsg::Offer { rem_pub, offer }) => {
                            let offer = Buf::from_json(offer)?;
                            sig_state.offer(rem_pub, offer)
                        }
                        Some(tx5_signal::SignalMsg::Answer { rem_pub, answer }) => {
                            let answer = Buf::from_json(answer)?;
                            sig_state.answer(rem_pub, answer)
                        }
                        Some(tx5_signal::SignalMsg::Ice { rem_pub, ice }) => {
                            let ice = Buf::from_json(ice)?;
                            sig_state.ice(rem_pub, ice)
                        }
                        None => Err(Error::id("SigClosed")),
                    }
                }.await {
                    sig_state.close(err);
                    break;
                }
            }
            msg = sig_evt.recv() => {
                match msg {
                    Some(Ok(state::SigStateEvt::SndOffer(
                        rem_id,
                        mut offer,
                        mut resp,
                    ))) => {
                        resp.with(move || async move {
                            sig.offer(rem_id, offer.to_json()?).await
                        }).await;
                    }
                    Some(Ok(state::SigStateEvt::SndAnswer(
                        rem_id,
                        mut answer,
                        mut resp,
                    ))) => {
                        resp.with(move || async move {
                            sig.answer(rem_id, answer.to_json()?).await
                        }).await;
                    }
                    Some(Ok(state::SigStateEvt::SndIce(
                        rem_id,
                        mut ice,
                        mut resp,
                    ))) => {
                        resp.with(move || async move {
                            sig.ice(rem_id, ice.to_json()?).await
                        }).await;
                    }
                    Some(Ok(state::SigStateEvt::SndDemo)) => {
                        sig.demo()
                    }
                    Some(Err(_)) => break,
                    None => break,
                }
            }
        };
    }
}

#[cfg(feature = "backend-go-pion")]
pub(crate) fn on_new_conn(
    config: DynConfig,
    ice_servers: serde_json::Value,
    seed: state::ConnStateSeed,
) {
    tokio::task::spawn(new_conn_task(config, ice_servers, seed));
}

#[cfg(feature = "backend-go-pion")]
async fn new_conn_task(
    _config: DynConfig,
    ice_servers: serde_json::Value,
    seed: state::ConnStateSeed,
) {
    use tx5_go_pion::DataChannelEvent as DataEvt;
    use tx5_go_pion::PeerConnectionEvent as PeerEvt;

    enum MultiEvt {
        Peer(PeerEvt),
        Data(DataEvt),
    }

    let (peer_snd, mut peer_rcv) = tokio::sync::mpsc::unbounded_channel();

    let peer_snd2 = peer_snd.clone();
    let mut peer = match async {
        let peer_config = Buf::from_json(ice_servers)?;

        let peer =
            tx5_go_pion::PeerConnection::new(peer_config.imp.buf, move |evt| {
                let _ = peer_snd2.send(MultiEvt::Peer(evt));
            })
            .await?;

        Result::Ok(peer)
    }
    .await
    {
        Ok(r) => r,
        Err(err) => {
            seed.result_err(err);
            return;
        }
    };

    let (conn_state, mut conn_evt) = match seed.result_ok() {
        Err(_) => return,
        Ok(r) => r,
    };

    let mut data_chan: Option<tx5_go_pion::DataChannel> = None;

    loop {
        tokio::select! {
            msg = peer_rcv.recv() => {
                match msg {
                    None => {
                        conn_state.close(Error::id("PeerConClosed"));
                        break;
                    }
                    Some(MultiEvt::Peer(PeerEvt::Error(err))) => {
                        conn_state.close(err);
                        break;
                    }
                    Some(MultiEvt::Peer(PeerEvt::ICECandidate(buf))) => {
                        let buf = Buf::from_raw(buf);
                        if conn_state.ice(buf).is_err() {
                            break;
                        }
                    }
                    Some(MultiEvt::Peer(PeerEvt::DataChannel(chan))) => {
                        let peer_snd = peer_snd.clone();
                        data_chan = Some(chan.handle(move |evt| {
                            let _ = peer_snd.send(MultiEvt::Data(evt));
                        }));
                    }
                    Some(MultiEvt::Data(DataEvt::Open)) => {
                        if conn_state.ready().is_err() {
                            break;
                        }
                    }
                    Some(MultiEvt::Data(DataEvt::Close)) => {
                        conn_state.close(Error::id("DataChanClosed"));
                        break;
                    }
                    Some(MultiEvt::Data(DataEvt::Message(buf))) => {
                        if conn_state.rcv_data(Buf::from_raw(buf)).is_err() {
                            break;
                        }
                    }
                }
            }
            msg = conn_evt.recv() => {
                match msg {
                    Some(Ok(state::ConnStateEvt::CreateOffer(mut resp))) => {
                        let peer = &mut peer;
                        let data_chan = &mut data_chan;
                        let peer_snd = peer_snd.clone();
                        resp.with(move || async move {
                            let chan = peer.create_data_channel(
                                tx5_go_pion::DataChannelConfig {
                                    label: Some("data".into()),
                                }
                            ).await?;

                            *data_chan = Some(chan.handle(move |evt| {
                                let _ = peer_snd.send(MultiEvt::Data(evt));
                            }));

                            let mut buf = peer.create_offer(
                                tx5_go_pion::OfferConfig::default(),
                            ).await?;

                            if let Ok(bytes) = buf.to_vec() {
                                tracing::debug!(
                                    offer=%String::from_utf8_lossy(&bytes),
                                    "create_offer",
                                );
                            }

                            Ok(Buf::from_raw(buf))
                        }).await;
                    }
                    Some(Ok(state::ConnStateEvt::CreateAnswer(mut resp))) => {
                        let peer = &mut peer;
                        resp.with(move || async move {

                            let mut buf = peer.create_answer(
                                tx5_go_pion::AnswerConfig::default(),
                            ).await?;
                            if let Ok(bytes) = buf.to_vec() {
                                tracing::debug!(
                                    offer=%String::from_utf8_lossy(&bytes),
                                    "create_answer",
                                );
                            }
                            Ok(Buf::from_raw(buf))
                        }).await;
                    }
                    Some(Ok(state::ConnStateEvt::SetLoc(buf, mut resp))) => {
                        let peer = &mut peer;
                        resp.with(move || async move {
                            peer.set_local_description(buf.imp.buf).await
                        }).await;
                    }
                    Some(Ok(state::ConnStateEvt::SetRem(buf, mut resp))) => {
                        let peer = &mut peer;
                        resp.with(move || async move {
                            peer.set_remote_description(buf.imp.buf).await
                        }).await;
                    }
                    Some(Ok(state::ConnStateEvt::SetIce(buf, mut resp))) => {
                        let peer = &mut peer;
                        resp.with(move || async move {
                            peer.add_ice_candidate(buf.imp.buf).await
                        }).await;
                    }
                    Some(Ok(state::ConnStateEvt::SndData(buf, mut resp))) => {
                        let data_chan = &mut data_chan;
                        resp.with(move || async move {
                            match data_chan {
                                None => Err(Error::id("NoDataChannel")),
                                Some(chan) => {
                                    chan.send(buf.imp.buf).await?;
                                    // TODO - actually report this
                                    Ok(state::BufState::Low)
                                }
                            }
                        }).await;
                    }
                    Some(Err(_)) => break,
                    None => break,
                }
            }
        };
    }
}
