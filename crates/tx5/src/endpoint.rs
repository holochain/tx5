//! Tx5 endpoint.

use crate::*;
use std::sync::Arc;
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

    /// Close down all connections to, fail all outgoing messages to,
    /// and drop all incoming messages from, the given remote id,
    /// for the specified ban time period.
    pub fn ban(&self, rem_id: Id, span: std::time::Duration) {
        self.state.ban(rem_id, span);
    }

    /// Send data to a remote on this tx5 endpoint.
    /// The future returned from this method will resolve when
    /// the data is handed off to our networking backend.
    pub fn send<B: bytes::Buf>(
        &self,
        rem_cli_url: Tx5Url,
        data: B,
    ) -> impl std::future::Future<Output = Result<()>> + 'static + Send {
        self.state.snd_data(rem_cli_url, data)
    }

    /// Broadcast data to all connections that happen to be open.
    /// If no connections are open, no data will be broadcast.
    /// The future returned from this method will resolve when all
    /// broadcast messages have been handed off to our networking backend.
    ///
    /// This method is currently not ideal. It naively gets a list
    /// of open connection urls and adds the broadcast to all of their queues.
    /// This could result in a connection being re-established just
    /// for the broadcast to occur.
    pub fn broadcast<B: bytes::Buf>(
        &self,
        mut data: B,
    ) -> impl std::future::Future<Output = Result<Vec<Result<()>>>> + 'static + Send
    {
        let data = data.copy_to_bytes(data.remaining());
        let state = self.state.clone();
        async move {
            let url_list = state.list_connected().await?;
            Ok(futures::future::join_all(
                url_list
                    .into_iter()
                    .map(|url| state.snd_data(url, data.clone())),
            )
            .await)
        }
    }

    /// Send a demo broadcast to every connected signal server.
    /// Warning, if demo mode is not enabled on these servers, this
    /// could result in a ban.
    pub fn demo(&self) -> Result<()> {
        self.state.snd_demo()
    }

    /// Get stats.
    pub fn get_stats(
        &self,
    ) -> impl std::future::Future<Output = Result<serde_json::Value>> + 'static + Send
    {
        self.state.stats()
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

    let ice_servers = sig.ice_servers();

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
                            let offer = BackBuf::from_json(offer)?;
                            sig_state.offer(rem_pub, offer)
                        }
                        Some(tx5_signal::SignalMsg::Answer { rem_pub, answer }) => {
                            let answer = BackBuf::from_json(answer)?;
                            sig_state.answer(rem_pub, answer)
                        }
                        Some(tx5_signal::SignalMsg::Ice { rem_pub, ice }) => {
                            let ice = BackBuf::from_json(ice)?;
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

    tracing::warn!("signal connection CLOSED");
}

pub(crate) fn on_new_conn(
    config: DynConfig,
    ice_servers: Arc<serde_json::Value>,
    seed: state::ConnStateSeed,
) {
    tokio::task::spawn(new_conn_task(config, ice_servers, seed));
}

#[cfg(feature = "backend-webrtc-rs")]
async fn new_conn_task(
    _config: DynConfig,
    ice_servers: Arc<serde_json::Value>,
    seed: state::ConnStateSeed,
) {
    use std::time::Duration;
    use webrtc::{
        api::APIBuilder,
        data_channel::{
            data_channel_message::DataChannelMessage, RTCDataChannel,
        },
        ice_transport::ice_server::RTCIceServer,
        peer_connection::{
            configuration::RTCConfiguration,
            peer_connection_state::RTCPeerConnectionState,
        },
    };

    // Create the API object

    let api = APIBuilder::new().build();

    // Prepare the configuration
    let config = RTCConfiguration {
        ice_servers: vec![RTCIceServer {
            urls: ice_servers
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap().to_owned())
                .collect(),
            ..Default::default()
        }],
        ..Default::default()
    };

    // Create a new RTCPeerConnection
    match api.new_peer_connection(config).await {
        Ok(r) => {
            let peer_connection = Arc::new(r);

            let (conn_state, mut conn_evt) = match seed.result_ok() {
                Err(_) => return,
                Ok(r) => r,
            };

            // Set the handler for Peer connection state
            // This will notify you when the peer has connected/disconnected
            peer_connection.on_peer_connection_state_change(Box::new(
                move |peer_connection_state: RTCPeerConnectionState| {
                    match peer_connection_state {
                        // RTCPeerConnectionState::Closed => {}
                        // RTCPeerConnectionState::Connected => {}
                        // RTCPeerConnectionState::Disconnected => {}
                        // RTCPeerConnectionState::Failed => {}
                        // RTCPeerConnectionState::New => {}
                        // RTCPeerConnectionState::Connecting => {}
                        // RTCPeerConnectionState::Unspecified => {}
                        RTCPeerConnectionState::New
                        | RTCPeerConnectionState::Connecting
                        | RTCPeerConnectionState::Connected
                        | RTCPeerConnectionState::Unspecified => {
                            tracing::debug!(?peer_connection_state);
                        }
                        RTCPeerConnectionState::Disconnected
                        | RTCPeerConnectionState::Failed
                        | RTCPeerConnectionState::Closed => {
                            conn_state.close(Error::err(format!(
                                "BackendState:{peer_connection_state:?}"
                            )));
                        }
                    }
                    Box::pin(async {})
                },
            ));

            // Register data channel creation handling
            peer_connection
                .on_data_channel(Box::new(move |data_channel: Arc<RTCDataChannel>| {
                    let data_channel_label = data_channel.label().to_owned();
                    let data_channel_id = data_channel.id();
                    println!("New DataChannel {data_channel_label} {data_channel_id}");
                    // Register channel opening handling
                    Box::pin(async move {
                        // let d2 = Arc::clone(&d);
                        // let d_label2 = data_channel_label.clone();
                        // let data_channel_id2 = data_channel_id;
                        data_channel.on_open(Box::new(move || {
                            Box::pin(async move {
                              // result = d2.send_text(message).await.map_err(Into::into);   
                            })
                        }));

                        // Register text message handling
                        data_channel.on_message(Box::new(move |msg: DataChannelMessage| {
                            let msg_str = String::from_utf8(msg.data.to_vec()).unwrap();
                            println!("Message from DataChannel '{data_channel_label}': '{msg_str}'");
                            Box::pin(async {})
                        }));
                    })
                }));
        }
        Err(_err) => {
            // seed.result_err(err);
            return;
        }
    };
}

#[cfg(feature = "backend-go-pion")]
async fn new_conn_task(
    _config: DynConfig,
    ice_servers: Arc<serde_json::Value>,
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
        let peer_config = BackBuf::from_json(ice_servers)?;

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

    tracing::debug!("PEER CON OPEN");

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
                        let buf = BackBuf::from_raw(buf);
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
                        if conn_state.rcv_data(BackBuf::from_raw(buf)).is_err() {
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

                            Ok(BackBuf::from_raw(buf))
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
                            Ok(BackBuf::from_raw(buf))
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
                    Some(Ok(state::ConnStateEvt::Stats(mut resp))) => {
                        let peer = &mut peer;
                        resp.with(move || async move {
                            peer.stats().await
                                .map(BackBuf::from_raw)
                        }).await;
                    }
                    Some(Err(_)) => break,
                    None => break,
                }
            }
        };
    }

    tracing::debug!("PEER CON CLOSE");
}
