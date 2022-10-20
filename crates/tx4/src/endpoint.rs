//! Tx4 endpoint.

//use crate::deps::lair_keystore_api::prelude::*;
use crate::*;
//use std::collections::HashMap;
use std::sync::Arc;
use tx4_core::Tx4Url;

/// EpEvt
pub enum EpEvt {
    /// Received data from a remote.
    Data {
        /// The remote client url that sent this message.
        rem_cli_url: Tx4Url,

        /// The payload of the message.
        data: Buf,

        /// Drop this when you've accepted the data to allow additional
        /// incoming messages.
        permit: state::Permit,
    },
}

/// Ep
pub struct Ep {
    state: state::State,
}

impl Ep {
    /// Construct a new tx4 endpoint.
    pub async fn new() -> Result<(Self, actor::ManyRcv<EpEvt>)> {
        Self::with_config(DefConfig::default()).await
    }

    /// Construct a new tx4 endpoint with configuration.
    pub async fn with_config<C: Config, I: IntoConfig<Config = C>>(
        into_config: I,
    ) -> Result<(Self, actor::ManyRcv<EpEvt>)> {
        let (ep_snd, ep_rcv) = tokio::sync::mpsc::unbounded_channel();

        let config = Arc::new(into_config.into_config().await?);
        let (state, mut state_evt) = state::State::new();
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
        sig_url: Tx4Url,
    ) -> impl std::future::Future<Output = Result<Tx4Url>> + 'static + Send
    {
        self.state.listener_sig(sig_url)
    }

    /// Send data to a remote on this tx4 endpoint.
    pub fn send(
        &self,
        rem_cli_url: Tx4Url,
        data: Buf,
    ) -> impl std::future::Future<Output = Result<()>> + 'static + Send {
        self.state.snd_data(rem_cli_url, data)
    }
}

pub(crate) fn on_new_sig(
    config: Arc<DefConfigBuilt>,
    sig_url: Tx4Url,
    seed: state::SigStateSeed,
) {
    tokio::task::spawn(new_sig_task(config, sig_url, seed));
}

async fn new_sig_task(
    config: Arc<DefConfigBuilt>,
    sig_url: Tx4Url,
    seed: state::SigStateSeed,
) {
    let (sig_snd, mut sig_rcv) = tokio::sync::mpsc::unbounded_channel();

    let (sig, cli_url) = match async {
        let sig = tx4_signal::Cli::builder()
            .with_lair_client(config.lair_client().clone())
            .with_lair_tag(config.lair_tag().clone())
            .with_url(sig_url.to_string().parse().unwrap())
            .with_recv_cb(move |msg| {
                let _ = sig_snd.send(msg);
            })
            .build()
            .await?;

        let cli_url = Tx4Url::new(sig.local_addr())?;

        Result::Ok((sig, cli_url))
    }
    .await
    {
        Ok(r) => r,
        Err(err) => {
            seed.result_err(err);
            return;
        }
    };

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
                        Some(tx4_signal::SignalMsg::Demo { .. }) => Ok(()),
                        Some(tx4_signal::SignalMsg::Offer { rem_pub, offer }) => {
                            let offer = Buf::from_json(offer)?;
                            sig_state.offer(rem_pub, offer)
                        }
                        Some(tx4_signal::SignalMsg::Answer { rem_pub, answer }) => {
                            let answer = Buf::from_json(answer)?;
                            sig_state.answer(rem_pub, answer)
                        }
                        Some(tx4_signal::SignalMsg::Ice { rem_pub, ice }) => {
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
                    Some(Err(_)) => break,
                    None => break,
                }
            }
        };
    }
}

#[cfg(feature = "backend-go-pion")]
pub(crate) fn on_new_conn(
    _config: Arc<DefConfigBuilt>,
    _ice_servers: serde_json::Value,
    _seed: state::ConnStateSeed,
) {
    todo!()
}

#[cfg(test)]
mod test {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn endpoint_sanity() {
        let mut srv_config = tx4_signal_srv::Config::default();
        srv_config.port = 0;
        //srv_config.ice_servers = serde_json::json!([]);
        srv_config.demo = true;

        let (addr, srv_driver) =
            tx4_signal_srv::exec_tx4_signal_srv(srv_config).unwrap();
        tokio::task::spawn(srv_driver);

        let sig_port = addr.port();

        // TODO remove
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let sig_url =
            Tx4Url::new(format!("ws://localhost:{}", sig_port)).unwrap();
        println!("sig_url: {}", sig_url);

        let (ep1, _ep_rcv1) = Ep::new().await.unwrap();

        let cli_url1 = ep1.listen(sig_url.clone()).await.unwrap();

        println!("cli_url1: {}", cli_url1);

        let (ep2, _ep_rcv2) = Ep::new().await.unwrap();

        let cli_url2 = ep2.listen(sig_url).await.unwrap();

        println!("cli_url2: {}", cli_url2);

        ep1.send(cli_url2, Buf::from_slice(b"hello").unwrap())
            .await
            .unwrap();
    }
}

/*
/// Tx4 Endpoint Event.
#[derive(Debug)]
pub enum EndpointEvent {
    /// An endpoint "listening" connection has been established.
    ListenerConnected(url::Url),
}

type EndpointEventCb = Arc<dyn Fn(EndpointEvent) + 'static + Send + Sync>;

/// Tx4 Endpoint.
pub struct Endpoint {
    cmd_send: tokio::sync::mpsc::UnboundedSender<Cmd>,
    lair_client: LairClient,
    lair_tag: Arc<str>,
}

impl Drop for Endpoint {
    fn drop(&mut self) {
        self.close();
    }
}

impl Endpoint {
    /// Construct a new endpoint instance suitable for listening for
    /// incoming connections through a signal server or establishing
    /// outgoing connections through a signal server.
    pub fn new<Cb>(
        lair_client: LairClient,
        lair_tag: Arc<str>,
        cb: Cb,
    ) -> Result<Self>
    where
        Cb: Fn(EndpointEvent) + 'static + Send + Sync,
    {
        let cb: EndpointEventCb = Arc::new(cb);
        let (cmd_send, cmd_recv) = tokio::sync::mpsc::unbounded_channel();
        tokio::task::spawn(endpoint_task(cmd_recv, cb));
        Ok(Self {
            cmd_send,
            lair_client,
            lair_tag,
        })
    }

    /// Shutdown this endpoint.
    pub fn close(&self) {
        let _ = self.cmd_send.send(Cmd::Shutdown);
    }

    /// List the urls this endpoint can be reached at through
    /// signal server listener connections.
    pub async fn url_list(&self) -> Result<Vec<url::Url>> {
        let (s, r) = tokio::sync::oneshot::channel();
        let _ = self.cmd_send.send(Cmd::UrlList(s));
        r.await.map_err(|_| Error::id("Shutdown"))
    }

    /// Connect as an endpoint intending to receive
    /// incoming connection through a tx4 signal server.
    pub async fn listen(&self, signal_url: url::Url) -> Result<()> {
        let mut bld = tx4_signal::CliBuilder::default();
        bld.set_lair_client(self.lair_client.clone());
        bld.set_lair_tag(self.lair_tag.clone());
        bld.set_url(signal_url.clone());

        {
            let signal_url = signal_url.clone();
            let cmd_send = self.cmd_send.clone();
            bld.set_recv_cb(move |evt| {
                let _ = cmd_send.send(Cmd::SigEvt(signal_url.clone(), evt));
            });
        }

        let cli = bld.build().await?;

        let (s, r) = tokio::sync::oneshot::channel();
        let _ = self.cmd_send.send(Cmd::NewSig(signal_url, cli, s));
        r.await.map_err(|_| Error::id("Shutdown"))
    }

    /// If we don't already have a connection, attempt to establish one.
    pub async fn connect(&self, peer_url: url::Url) -> Result<()> {
        let (s, r) = tokio::sync::oneshot::channel();
        let _ = self.cmd_send.send(Cmd::Connect(peer_url, s));
        r.await.map_err(|_| Error::id("Shutdown"))?
    }
}

enum Cmd {
    Shutdown,
    SigEvt(url::Url, tx4_signal::SignalMsg),
    NewSig(url::Url, tx4_signal::Cli, tokio::sync::oneshot::Sender<()>),
    UrlList(tokio::sync::oneshot::Sender<Vec<url::Url>>),
    Connect(url::Url, tokio::sync::oneshot::Sender<Result<()>>),
}

async fn endpoint_task(
    cmd_recv: tokio::sync::mpsc::UnboundedReceiver<Cmd>,
    cb: EndpointEventCb,
) {
    if let Err(err) = endpoint_task_inner(cmd_recv, cb).await {
        tracing::error!(?err);
    }
}

enum PeerState {
    Connecting,
}

async fn endpoint_task_inner(
    mut cmd_recv: tokio::sync::mpsc::UnboundedReceiver<Cmd>,
    cb: EndpointEventCb,
) -> Result<()> {
    let mut signals = HashMap::new();
    let mut peers = HashMap::new();

    while let Some(cmd) = cmd_recv.recv().await {
        match cmd {
            Cmd::Shutdown => break,
            #[allow(clippy::match_single_binding)] // obvs this is temp
            Cmd::SigEvt(_url, evt) => match evt {
                _ => todo!(),
            },
            Cmd::NewSig(url, cli, resp) => {
                let addr = cli.local_addr().clone();
                signals.insert(url, Arc::new(cli));
                cb(EndpointEvent::ListenerConnected(addr));
                let _ = resp.send(());
            }
            Cmd::UrlList(resp) => {
                let mut out = Vec::new();
                for (_, cli) in signals.iter() {
                    out.push(cli.local_addr().clone());
                }
                let _ = resp.send(out);
            }
            Cmd::Connect(url, resp) => {
                let front = match url::Url::parse(&format!(
                    "{}://{}:{}",
                    url.scheme(),
                    url.host_str().unwrap_or("localhost"),
                    url.port().unwrap_or(5000),
                )) {
                    Err(err) => {
                        let _ = resp.send(Err(Error::err(err)));
                        continue;
                    }
                    Ok(front) => front,
                };

                if !signals.contains_key(&front) {
                    let _ =
                        resp.send(Err(Error::id("UnimplementedAutoSignal")));
                    continue;
                }

                // TODO - fixme unwraps
                let mut path = url.path_segments().unwrap();
                path.next().unwrap();
                let id =
                    tx4_signal::Id::from_b64(path.next().unwrap()).unwrap();

                if peers.contains_key(&id) {
                    let _ = resp.send(Ok(()));
                    continue;
                }

                peers.insert(id, PeerState::Connecting);

                // unwrap ok, checked above
                let _cli = signals.get(&front).unwrap().clone();

                let _ = resp.send(Err(Error::id("todo")));
            }
        }
    }

    Ok(())
}

//async fn connect_task(
//    id: tx4_signal::Id,
//    cli: Arc<tx4_signal::Cli>,
*/
