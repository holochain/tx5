use crate::*;
use futures::sink::SinkExt;
use futures::stream::StreamExt;
use lair_keystore_api::prelude::*;
use std::future::Future;
use std::sync::Arc;
use tokio_tungstenite::tungstenite::Message;
use tx4_core::wire;

type Socket = tokio_tungstenite::WebSocketStream<
    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
>;

type RecvCb = Box<dyn FnMut(wire::Wire) + 'static + Send>;

/// Builder for constructing a Cli instance.
pub struct CliBuilder {
    tls: Option<tls::TlsConfig>,
    recv_cb: RecvCb,
    lair_client: Option<LairClient>,
    lair_tag: Option<Arc<str>>,
    url: Option<url::Url>,
}

impl Default for CliBuilder {
    fn default() -> Self {
        Self {
            tls: None,
            recv_cb: Box::new(|_| {}),
            lair_client: None,
            lair_tag: None,
            url: None,
        }
    }
}

impl CliBuilder {
    /// Set the TlsConfig.
    pub fn set_tls(&mut self, tls: Option<tls::TlsConfig>) {
        self.tls = tls;
    }

    /// Apply a TlsConfig.
    pub fn with_tls(mut self, tls: Option<tls::TlsConfig>) -> Self {
        self.set_tls(tls);
        self
    }

    /// Set the receiver callback.
    pub fn set_recv_cb<Cb>(&mut self, cb: Cb)
    where
        Cb: FnMut(wire::Wire) + 'static + Send,
    {
        self.recv_cb = Box::new(cb);
    }

    /// Apply the receiver callback.
    pub fn with_recv_cb<Cb>(mut self, cb: Cb) -> Self
    where
        Cb: FnMut(wire::Wire) + 'static + Send,
    {
        self.set_recv_cb(cb);
        self
    }

    /// Set the LairClient.
    pub fn set_lair_client(&mut self, lair_client: LairClient) {
        self.lair_client = Some(lair_client);
    }

    /// Apply the LairClient.
    pub fn with_lair_client(mut self, lair_client: LairClient) -> Self {
        self.set_lair_client(lair_client);
        self
    }

    /// Set the Lair tag.
    pub fn set_lair_tag(&mut self, lair_tag: Arc<str>) {
        self.lair_tag = Some(lair_tag);
    }

    /// Apply the Lair tag.
    pub fn with_lair_tag(mut self, lair_tag: Arc<str>) -> Self {
        self.set_lair_tag(lair_tag);
        self
    }

    /// Set the server url.
    pub fn set_url(&mut self, url: url::Url) {
        self.url = Some(url);
    }

    /// Apply the server url.
    pub fn with_url(mut self, url: url::Url) -> Self {
        self.set_url(url);
        self
    }

    /// Build the Srv instance.
    pub async fn build(self) -> Result<Cli> {
        Cli::priv_build(self).await
    }
}

type WriteSend = tokio::sync::mpsc::Sender<(
    Vec<u8>,
    tokio::sync::oneshot::Sender<Result<()>>,
)>;

type WriteRecv = tokio::sync::mpsc::Receiver<(
    Vec<u8>,
    tokio::sync::oneshot::Sender<Result<()>>,
)>;

/// Tx4-signal client connection type.
pub struct Cli {
    addr: url::Url,
    hnd: tokio::task::JoinHandle<()>,
    ice: serde_json::Value,
    write_send: WriteSend,
}

impl Drop for Cli {
    fn drop(&mut self) {
        self.close();
    }
}

impl Cli {
    /// Get a CliBuilder.
    pub fn builder() -> CliBuilder {
        CliBuilder::default()
    }

    /// Shutdown this client instance.
    pub fn close(&self) {
        self.hnd.abort();
    }

    /// Get the addr this cli can be reached at through the signal server.
    pub fn local_addr(&self) -> &url::Url {
        &self.addr
    }

    /// Get the ice server list provided by the server.
    pub fn ice_servers(&self) -> &serde_json::Value {
        &self.ice
    }

    /// Send a message to the tx4-signal server.
    pub fn send(
        &self,
        wire: wire::Wire,
    ) -> impl Future<Output = Result<()>> + 'static + Send {
        let write_send = self.write_send.clone();
        async move {
            let wire = wire.encode()?;
            let (s, r) = tokio::sync::oneshot::channel();
            write_send
                .send((wire, s))
                .await
                .map_err(|_| Error::id("ClientClosed"))?;
            r.await.map_err(|_| Error::id("ClientClosed"))?
        }
    }

    // -- private -- //

    async fn priv_build(builder: CliBuilder) -> Result<Self> {
        let CliBuilder {
            tls,
            recv_cb,
            lair_client,
            lair_tag,
            url,
        } = builder;

        let lair_client = match lair_client {
            Some(lair_client) => lair_client,
            None => return Err(Error::id("LairClientRequired")),
        };

        let lair_tag = match lair_tag {
            Some(lair_tag) => lair_tag,
            None => return Err(Error::id("LairTagRequired")),
        };

        let url = match url {
            Some(url) => url,
            None => return Err(Error::id("UrlRequired")),
        };

        let x25519_pub = match lair_client.get_entry(lair_tag).await {
            Ok(LairEntryInfo::Seed { tag: _, seed_info }) => {
                Id::from_slice(&*seed_info.x25519_pub_key.0)?
            }
            _ => return Err(Error::err("lair_tag invalid seed")),
        };

        let use_tls = match url.scheme() {
            "ws" => false,
            "wss" => true,
            _ => {
                return Err(Error::err(format!(
                    "invalid scheme, expected \"ws\" or \"wss\", got {:?}",
                    url.scheme()
                )));
            }
        };

        if use_tls && tls.is_none() {
            return Err(Error::err("tls required but no tls config supplied"));
        }

        tracing::debug!(?use_tls, %url, ?x25519_pub);

        let host = match url.host_str() {
            None => return Err(Error::id("InvalidHost")),
            Some(host) => host,
        };

        let port = url.port().unwrap_or(443);

        let endpoint = format!("{}:{}", host, port);

        let con_url = if use_tls {
            format!("wss://{}/{}", endpoint, x25519_pub)
        } else {
            format!("ws://{}/{}", endpoint, x25519_pub)
        };

        let mut err_list = Vec::new();
        let mut result_socket = None;

        for addr in tokio::net::lookup_host(&endpoint).await? {
            match Self::priv_con(use_tls, &tls, &host, &con_url, addr).await {
                Ok(con) => {
                    result_socket = Some(con);
                    break;
                }
                Err(err) => {
                    err_list.push(format!("{:?}", err));
                    continue;
                }
            }
        }

        let mut socket = match result_socket {
            Some(socket) => socket,
            None => return Err(Error::err(format!("{:?}", err_list))),
        };

        let auth_req = match socket.next().await {
            Some(Ok(auth_req)) => auth_req.into_data(),
            Some(Err(err)) => return Err(Error::err(err)),
            None => return Err(Error::id("InvalidServerAuthReq")),
        };

        let (srv_pub, nonce, cipher, ice) = match wire::Wire::decode(&auth_req)?
        {
            wire::Wire::AuthReqV1 {
                srv_pub,
                nonce,
                cipher,
                ice,
            } => (srv_pub, nonce, cipher, ice),
            _ => return Err(Error::id("InvalidServerAuthReq")),
        };

        let con_key = lair_client
            .crypto_box_xsalsa_open_by_pub_key(
                srv_pub.0.into(),
                x25519_pub.0.into(),
                None,
                nonce.0.into(),
                cipher.0.into(),
            )
            .await?;

        socket
            .send(Message::binary(
                wire::Wire::AuthResV1 {
                    con_key: Id::from_slice(&con_key)?,
                    req_addr: true,
                }
                .encode()?,
            ))
            .await
            .map_err(Error::err)?;

        let url = url::Url::parse(&con_url).map_err(Error::err)?;

        tracing::debug!(%url);

        let (write_send, write_recv) = tokio::sync::mpsc::channel(1);

        let hnd = tokio::task::spawn(con_task(socket, recv_cb, write_recv));

        Ok(Self {
            addr: url,
            hnd,
            ice,
            write_send,
        })
    }

    async fn priv_con(
        use_tls: bool,
        tls: &Option<crate::tls::TlsConfig>,
        host: &str,
        con_url: &str,
        addr: std::net::SocketAddr,
    ) -> Result<Socket> {
        tracing::debug!(?addr, "try connect");

        let socket = match tokio::net::TcpStream::connect(addr).await {
            Ok(socket) => socket,
            Err(err) => return Err(err),
        };

        let socket = match tcp_configure(socket) {
            Ok(socket) => socket,
            Err(err) => return Err(err),
        };

        let socket: tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream> =
            if use_tls {
                let name = host
                    .try_into()
                    .unwrap_or_else(|_| "tx4-signal".try_into().unwrap());

                let socket = match tokio_rustls::TlsConnector::from(
                    tls.as_ref().unwrap().cli.clone(),
                )
                .connect(name, socket)
                .await
                {
                    Ok(socket) => socket.into(),
                    Err(err) => return Err(err),
                };

                tokio_tungstenite::MaybeTlsStream::Rustls(socket)
            } else {
                tokio_tungstenite::MaybeTlsStream::Plain(socket)
            };

        let (socket, _rsp) = match tokio_tungstenite::client_async_with_config(
            con_url,
            socket,
            Some(WS_CONFIG),
        )
        .await
        .map_err(Error::err)
        {
            Ok(r) => r,
            Err(err) => return Err(err),
        };

        Ok(socket)
    }
}

async fn con_task(socket: Socket, recv_cb: RecvCb, write_recv: WriteRecv) {
    if let Err(err) = con_task_err(socket, recv_cb, write_recv).await {
        tracing::error!(?err);
    }
}

async fn con_task_err(
    socket: Socket,
    mut recv_cb: RecvCb,
    mut write_recv: WriteRecv,
) -> Result<()> {
    let (mut write, mut read) = socket.split();

    tokio::select! {
        r = async move {
            while let Some(msg) = read.next().await {
                let msg = msg.map_err(Error::err)?.into_data();
                match wire::Wire::decode(&msg)? {
                    msg @ wire::Wire::OfferV1 { .. } |
                        msg @ wire::Wire::AnswerV1 { .. } |
                        msg @ wire::Wire::IceV1 { .. } |
                        msg @ wire::Wire::DemoV1 { .. }
                    => {
                        recv_cb(msg)
                    }
                    _ => return Err(Error::id("InvalidClientMsg")),
                }
            }

            Ok(())
        } => r,

        r = async move {
            while let Some((msg, resp)) = write_recv.recv().await {
                if let Err(err) = write.send(Message::binary(msg)).await.map_err(Error::err) {
                    let _ = resp.send(Err(err.err_clone()));
                    return Err(err);
                }
                let _ = resp.send(Ok(()));
            }

            Ok(())
        } => r,
    }
}
