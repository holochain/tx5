//! Tx4 endpoint.

use crate::deps::lair_keystore_api::prelude::*;
use crate::*;
use std::collections::HashMap;
use std::sync::Arc;

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
