//! Tx5 endpoint.

use crate::*;
use opentelemetry_api::{metrics::Unit, KeyValue};
use std::collections::HashMap;
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

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct BackendMetrics {
    #[serde(default)]
    messages_sent: u64,
    #[serde(default)]
    messages_received: u64,
    #[serde(default)]
    bytes_sent: u64,
    #[serde(default)]
    bytes_received: u64,
}

#[cfg(feature = "backend-go-pion")]
pub(crate) fn on_new_conn(
    config: DynConfig,
    ice_servers: Arc<serde_json::Value>,
    seed: state::ConnStateSeed,
) {
    tokio::task::spawn(new_conn_task(config, ice_servers, seed));
}

#[cfg(feature = "backend-go-pion")]
async fn new_conn_task(
    _config: DynConfig,
    ice_servers: Arc<serde_json::Value>,
    seed: state::ConnStateSeed,
) {
    use tx5_go_pion::DataChannelEvent as DataEvt;
    use tx5_go_pion::PeerConnectionEvent as PeerEvt;
    use tx5_go_pion::PeerConnectionState as PeerState;

    enum MultiEvt {
        Stats(
            tokio::sync::oneshot::Sender<
                Option<HashMap<String, BackendMetrics>>,
            >,
        ),
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

    let state_uniq = conn_state.meta().state_uniq.clone();
    let conn_uniq = conn_state.meta().conn_uniq.clone();
    let rem_id = conn_state.meta().cli_url.id().unwrap();

    struct Unregister(
        Option<Box<dyn opentelemetry_api::metrics::CallbackRegistration>>,
    );
    impl Drop for Unregister {
        fn drop(&mut self) {
            if let Some(mut unregister) = self.0.take() {
                let _ = unregister.unregister();
            }
        }
    }

    let slot: Arc<std::sync::Mutex<Option<HashMap<String, BackendMetrics>>>> =
        Arc::new(std::sync::Mutex::new(None));
    let weak_slot = Arc::downgrade(&slot);
    let peer_snd_task = peer_snd.clone();
    tokio::task::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;

            if let Some(slot) = weak_slot.upgrade() {
                let (s, r) = tokio::sync::oneshot::channel();
                if peer_snd_task.send(MultiEvt::Stats(s)).is_err() {
                    break;
                }
                if let Ok(stats) = r.await {
                    *slot.lock().unwrap() = stats;
                }
            } else {
                break;
            }
        }
    });

    let weak_slot = Arc::downgrade(&slot);
    let _unregister = {
        use opentelemetry_api::metrics::MeterProvider;

        let meter = opentelemetry_api::global::meter_provider()
            .versioned_meter(
                "tx5",
                None::<&'static str>,
                None::<&'static str>,
                Some(vec![
                    KeyValue::new("state_uniq", state_uniq.to_string()),
                    KeyValue::new("conn_uniq", conn_uniq.to_string()),
                    KeyValue::new("remote_id", rem_id.to_string()),
                ]),
            );
        let ice_snd = meter
            .u64_observable_counter("tx5.conn.ice.send")
            .with_description("Bytes sent on ice channel")
            .with_unit(Unit::new("By"))
            .init();
        let ice_rcv = meter
            .u64_observable_counter("tx5.conn.ice.recv")
            .with_description("Bytes received on ice channel")
            .with_unit(Unit::new("By"))
            .init();
        let data_snd = meter
            .u64_observable_counter("tx5.conn.data.send")
            .with_description("Bytes sent on data channel")
            .with_unit(Unit::new("By"))
            .init();
        let data_rcv = meter
            .u64_observable_counter("tx5.conn.data.recv")
            .with_description("Bytes received on data channel")
            .with_unit(Unit::new("By"))
            .init();
        let data_snd_msg = meter
            .u64_observable_counter("tx5.conn.data.send.message.count")
            .with_description("Message count sent on data channel")
            .init();
        let data_rcv_msg = meter
            .u64_observable_counter("tx5.conn.data.recv.message.count")
            .with_description("Message count received on data channel")
            .init();
        let unregister = match meter.register_callback(
            &[data_snd.as_any(), data_rcv.as_any()],
            move |obs| {
                if let Some(slot) = weak_slot.upgrade() {
                    if let Some(slot) = slot.lock().unwrap().take() {
                        for (k, v) in slot.iter() {
                            if k.starts_with("DataChannel") {
                                obs.observe_u64(&data_snd, v.bytes_sent, &[]);
                                obs.observe_u64(
                                    &data_rcv,
                                    v.bytes_received,
                                    &[],
                                );
                                obs.observe_u64(
                                    &data_snd_msg,
                                    v.messages_sent,
                                    &[],
                                );
                                obs.observe_u64(
                                    &data_rcv_msg,
                                    v.messages_received,
                                    &[],
                                );
                            } else if k.starts_with("iceTransport") {
                                obs.observe_u64(&ice_snd, v.bytes_sent, &[]);
                                obs.observe_u64(
                                    &ice_rcv,
                                    v.bytes_received,
                                    &[],
                                );
                            }
                        }
                    }
                }
            },
        ) {
            Ok(unregister) => Some(unregister),
            Err(err) => {
                tracing::warn!(?err, "unable to register connection metrics");
                None
            }
        };
        Unregister(unregister)
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
                    Some(MultiEvt::Stats(resp)) => {
                        if let Ok(mut buf) = peer.stats().await.map(BackBuf::from_raw) {
                            if let Ok(val) = buf.to_json() {
                                let _ = resp.send(Some(val));
                            } else {
                                let _ = resp.send(None);
                            }
                        } else {
                            let _ = resp.send(None);
                        }
                    }
                    Some(MultiEvt::Peer(PeerEvt::Error(err))) => {
                        conn_state.close(err);
                        break;
                    }
                    Some(MultiEvt::Peer(PeerEvt::State(peer_state))) => {
                        match peer_state {
                            PeerState::New
                            | PeerState::Connecting
                            | PeerState::Connected => {
                                tracing::debug!(?peer_state);
                            }
                            PeerState::Disconnected
                            | PeerState::Failed
                            | PeerState::Closed => {
                                conn_state.close(Error::err(format!("BackendState:{peer_state:?}")));
                                break;
                            }
                        }
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
