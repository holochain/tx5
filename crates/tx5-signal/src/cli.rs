use crate::*;
use futures::sink::SinkExt;
use futures::stream::StreamExt;
use lair_keystore_api::prelude::*;
use parking_lot::Mutex;
use std::future::Future;
use std::sync::atomic;
use std::sync::Arc;
use tokio_tungstenite::tungstenite::Message;
use tx5_core::wire;

type Socket = tokio_tungstenite::WebSocketStream<
    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
>;

const PROTO_VER: &str = "v1";

/// Incoming signal message from a remote node.
#[derive(Debug)]
pub enum SignalMsg {
    /// We received a demo broadcast from the signal server.
    Demo {
        /// The remote Id that is connected to the signal server.
        rem_pub: Id,
    },

    /// WebRTC offer.
    Offer {
        /// The remote Id sending the offer.
        rem_pub: Id,

        /// The WebRTC offer.
        offer: serde_json::Value,
    },

    /// WebRTC answer.
    Answer {
        /// The remote Id sending the answer.
        rem_pub: Id,

        /// The WebRTC answer.
        answer: serde_json::Value,
    },

    /// WebRTC ICE candidate.
    Ice {
        /// The remote Id sending the ICE candidate.
        rem_pub: Id,

        /// The WebRTC ICE candidate.
        ice: serde_json::Value,
    },
}

type RecvCb = Box<dyn FnMut(SignalMsg) + 'static + Send>;

/// Builder for constructing a Cli instance.
pub struct CliBuilder {
    recv_cb: RecvCb,
    lair_client: Option<LairClient>,
    lair_tag: Option<Arc<str>>,
    url: Option<url::Url>,
}

impl Default for CliBuilder {
    fn default() -> Self {
        Self {
            recv_cb: Box::new(|_| {}),
            lair_client: None,
            lair_tag: None,
            url: None,
        }
    }
}

impl CliBuilder {
    /// Set the receiver callback.
    pub fn set_recv_cb<Cb>(&mut self, cb: Cb)
    where
        Cb: FnMut(SignalMsg) + 'static + Send,
    {
        self.recv_cb = Box::new(cb);
    }

    /// Apply the receiver callback.
    pub fn with_recv_cb<Cb>(mut self, cb: Cb) -> Self
    where
        Cb: FnMut(SignalMsg) + 'static + Send,
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

fn priv_system_tls() -> Arc<rustls::ClientConfig> {
    let mut roots = rustls::RootCertStore::empty();
    for cert in rustls_native_certs::load_native_certs()
        .expect("failed to load system tls certs")
    {
        roots
            .add(&rustls::Certificate(cert.0))
            .expect("faild to add cert to root");
    }

    Arc::new(
        rustls::ClientConfig::builder()
            .with_safe_defaults()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    )
}

type Respond = tokio::sync::oneshot::Sender<Result<()>>;

type WriteSend = tokio::sync::mpsc::Sender<(Message, Respond)>;
type WriteRecv = tokio::sync::mpsc::Receiver<(Message, Respond)>;

struct Seq(atomic::AtomicU64);

impl Seq {
    pub const fn new() -> Self {
        Self(atomic::AtomicU64::new(0))
    }

    pub fn get(&self) -> f64 {
        let mut out = (std::time::SystemTime::UNIX_EPOCH
            .elapsed()
            .unwrap()
            .as_secs_f64()
            * 1000.0) as u64;
        self.0
            .fetch_update(
                atomic::Ordering::SeqCst,
                atomic::Ordering::SeqCst,
                |cur| {
                    if cur >= out {
                        out = cur + 1
                    }
                    Some(out)
                },
            )
            .unwrap();
        out as f64
    }
}

/// Tx5-signal client connection type.
pub struct Cli {
    addr: url::Url,
    hnd: Vec<tokio::task::JoinHandle<()>>,
    ice: Arc<Mutex<Arc<serde_json::Value>>>,
    write_send: WriteSend,
    seq: Seq,
    _lair_keystore: Option<lair_keystore_api::in_proc_keystore::InProcKeystore>,
    lair_client: LairClient,
    x25519_pub: Id,
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
        for h in self.hnd.iter() {
            h.abort();
        }
    }

    /// Get the id (x25519 public key) that this local node is identified by.
    pub fn local_id(&self) -> &Id {
        &self.x25519_pub
    }

    /// Get the addr this cli can be reached at through the signal server.
    pub fn local_addr(&self) -> &url::Url {
        &self.addr
    }

    /// Get the ice server list provided by the server.
    pub fn ice_servers(&self) -> Arc<serde_json::Value> {
        self.ice.lock().clone()
    }

    /// Send a WebRTC offer to a remote node on the signal server.
    pub fn offer(
        &self,
        rem_pub: Id,
        offer: serde_json::Value,
    ) -> impl Future<Output = Result<()>> + 'static + Send {
        self.priv_send(
            rem_pub,
            wire::FwdInnerV1::Offer {
                seq: 0.0, // set in priv_send
                offer,
            },
        )
    }

    /// Send a WebRTC answer to a remote node on the signal server.
    pub fn answer(
        &self,
        rem_pub: Id,
        answer: serde_json::Value,
    ) -> impl Future<Output = Result<()>> + 'static + Send {
        self.priv_send(
            rem_pub,
            wire::FwdInnerV1::Answer {
                seq: 0.0, // set in priv_send
                answer,
            },
        )
    }

    /// Send a WebRTC ICE candidate to a remote node on the signal server.
    pub fn ice(
        &self,
        rem_pub: Id,
        ice: serde_json::Value,
    ) -> impl Future<Output = Result<()>> + 'static + Send {
        self.priv_send(
            rem_pub,
            wire::FwdInnerV1::Ice {
                seq: 0.0, // set in priv_send
                ice,
            },
        )
    }

    /// Send a demo broadcast to the signal server.
    /// Warning, if demo mode is not enabled on this server,
    /// this could result in a ban.
    pub fn demo(&self) {
        let write_send = self.write_send.clone();
        let rem_pub = self.x25519_pub;
        tokio::task::spawn(async move {
            let (s, r) = tokio::sync::oneshot::channel();
            let _ = write_send
                .send((
                    Message::binary(
                        wire::Wire::DemoV1 { rem_pub }.encode().unwrap(),
                    ),
                    s,
                ))
                .await;
            let _ = r.await;
        });
    }

    // -- private -- //

    fn priv_send(
        &self,
        rem_pub: Id,
        mut msg: wire::FwdInnerV1,
    ) -> impl Future<Output = Result<()>> + 'static + Send {
        msg.set_seq(self.seq.get());

        let lair_client = self.lair_client.clone();
        let x25519_pub = self.x25519_pub;
        let write_send = self.write_send.clone();

        async move {
            let (nonce, cipher) = lair_client
                .crypto_box_xsalsa_by_pub_key(
                    x25519_pub.0.into(),
                    rem_pub.0.into(),
                    None,
                    msg.encode()?.into(),
                )
                .await?;

            let wire = wire::Wire::FwdV1 {
                rem_pub,
                nonce: nonce.into(),
                cipher: cipher.to_vec().into_boxed_slice().into(),
            }
            .encode()?;

            let (s, r) = tokio::sync::oneshot::channel();

            write_send
                .send((Message::binary(wire), s))
                .await
                .map_err(|_| Error::id("ClientClosed"))?;

            r.await.map_err(|_| Error::id("ClientClosed"))?
        }
    }

    async fn priv_build(builder: CliBuilder) -> Result<Self> {
        let CliBuilder {
            recv_cb,
            lair_client,
            lair_tag,
            url,
        } = builder;

        let mut lair_keystore = None;

        let lair_tag = match lair_tag {
            Some(lair_tag) => lair_tag,
            None => rand_utf8::rand_utf8(&mut rand::thread_rng(), 32).into(),
        };

        let lair_client = match lair_client {
            Some(lair_client) => lair_client,
            None => {
                let passphrase = sodoken::BufRead::new_no_lock(
                    rand_utf8::rand_utf8(&mut rand::thread_rng(), 32)
                        .as_bytes(),
                );

                // this is a memory keystore,
                // so weak persistence security is okay,
                // since it will not be persisted.
                // The private keys will still be mem_locked
                // so they shouldn't be swapped to disk.
                let keystore_config = PwHashLimits::Minimum
                    .with_exec(|| {
                        LairServerConfigInner::new("/", passphrase.clone())
                    })
                    .await
                    .unwrap();

                let keystore = PwHashLimits::Minimum
                    .with_exec(|| {
                        lair_keystore_api::in_proc_keystore::InProcKeystore::new(
                            Arc::new(keystore_config),
                            lair_keystore_api::mem_store::create_mem_store_factory(),
                            passphrase,
                        )
                    })
                    .await
                    .unwrap();

                let lair_client = keystore.new_client().await.unwrap();

                lair_client
                    .new_seed(lair_tag.clone(), None, false)
                    .await
                    .unwrap();

                lair_keystore = Some(keystore);

                lair_client
            }
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
            "ws" => None,
            "wss" => Some(priv_system_tls()),
            _ => {
                return Err(Error::err(format!(
                    "invalid scheme, expected \"ws\" or \"wss\", got {:?}",
                    url.scheme()
                )));
            }
        };

        tracing::debug!(use_tls=%use_tls.is_some(), %url, ?x25519_pub);

        let host = match url.host_str() {
            None => return Err(Error::id("InvalidHost")),
            Some(host) => host,
        };

        let port = url.port().unwrap_or(443);

        let endpoint = format!("{host}:{port}");

        let (con_url, con_url_versioned) = if use_tls.is_some() {
            (
                format!("wss://{endpoint}/tx5-ws/{x25519_pub}"),
                format!("wss://{endpoint}/tx5-ws/{PROTO_VER}/{x25519_pub}"),
            )
        } else {
            (
                format!("ws://{endpoint}/tx5-ws/{x25519_pub}"),
                format!("ws://{endpoint}/tx5-ws/{PROTO_VER}/{x25519_pub}"),
            )
        };

        let url = url::Url::parse(&con_url).map_err(Error::err)?;
        tracing::debug!(%url);

        let (write_send, write_recv) = tokio::sync::mpsc::channel(1);

        let mut hnd = Vec::with_capacity(2);

        let ice = Arc::new(Mutex::new(Arc::new(serde_json::json!({
            "iceServers": [],
        }))));

        let (init_send, init_recv) = tokio::sync::oneshot::channel();

        hnd.push(tokio::task::spawn(con_task(
            use_tls,
            host.to_string(),
            con_url_versioned,
            endpoint,
            ice.clone(),
            recv_cb,
            x25519_pub,
            lair_client.clone(),
            write_send.clone(),
            write_recv,
            init_send,
        )));

        let keep_alive = write_send.clone();
        hnd.push(tokio::task::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                let (s, r) = tokio::sync::oneshot::channel();
                if keep_alive
                    .send((Message::Ping(Vec::new()), s))
                    .await
                    .is_err()
                {
                    break;
                }
                match r.await {
                    Ok(Err(_)) | Err(_) => break,
                    Ok(Ok(_)) => (),
                }
            }
        }));

        let _ = init_recv.await;

        Ok(Self {
            addr: url,
            hnd,
            ice,
            write_send,
            seq: Seq::new(),
            _lair_keystore: lair_keystore,
            lair_client,
            x25519_pub,
        })
    }
}

#[allow(clippy::too_many_arguments)]
async fn con_task(
    use_tls: Option<Arc<rustls::ClientConfig>>,
    host: String,
    con_url: String,
    endpoint: String,
    ice: Arc<Mutex<Arc<serde_json::Value>>>,
    recv_cb: RecvCb,
    x25519_pub: Id,
    lair_client: LairClient,
    write_send: WriteSend,
    write_recv: WriteRecv,
    init: tokio::sync::oneshot::Sender<()>,
) {
    let mut init = Some(init);
    let mut recv_cb = Some(recv_cb);
    let mut write_recv = Some(write_recv);
    loop {
        if let Some(socket) = con_open_connection(
            &use_tls,
            &host,
            &con_url,
            &endpoint,
            x25519_pub,
            &ice,
            &lair_client,
        )
        .await
        {
            // once we've run open_connection once, proceed with init
            if let Some(init) = init.take() {
                let _ = init.send(());
            }

            let (a_recv_cb, a_write_recv) = con_manage_connection(
                socket,
                recv_cb.take().unwrap(),
                x25519_pub,
                &lair_client,
                write_send.clone(),
                write_recv.take().unwrap(),
            )
            .await;
            recv_cb = Some(a_recv_cb);
            write_recv = Some(a_write_recv);
        }

        // once we've run open_connection once, proceed with init
        if let Some(init) = init.take() {
            let _ = init.send(());
        }

        let s = rand::Rng::gen_range(&mut rand::thread_rng(), 4.0..8.0);
        let s = std::time::Duration::from_secs_f64(s);
        tokio::time::sleep(s).await;
    }
}

async fn con_stack(
    use_tls: &Option<Arc<rustls::ClientConfig>>,
    host: &str,
    con_url: &str,
    addr: std::net::SocketAddr,
) -> Option<Socket> {
    tracing::debug!(?addr, "try connect");

    let socket = match tokio::net::TcpStream::connect(addr).await {
        Ok(socket) => socket,
        Err(err) => {
            tracing::debug!(?err);
            return None;
        }
    };

    let socket = match tcp_configure(socket) {
        Ok(socket) => socket,
        Err(err) => {
            tracing::debug!(?err);
            return None;
        }
    };

    let socket: tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream> =
        if let Some(tls) = use_tls {
            let name = host
                .try_into()
                .unwrap_or_else(|_| "tx5-signal".try_into().unwrap());

            let socket = match tokio_rustls::TlsConnector::from(tls.clone())
                .connect(name, socket)
                .await
            {
                Ok(socket) => socket,
                Err(err) => {
                    tracing::debug!(?err);
                    return None;
                }
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
    {
        Ok(r) => r,
        Err(err) => {
            tracing::debug!(?err);
            return None;
        }
    };

    Some(socket)
}

async fn con_open_connection(
    use_tls: &Option<Arc<rustls::ClientConfig>>,
    host: &str,
    con_url: &str,
    endpoint: &str,
    x25519_pub: Id,
    ice: &Mutex<Arc<serde_json::Value>>,
    lair_client: &LairClient,
) -> Option<Socket> {
    let mut result_socket = None;

    let addr_list = match tokio::net::lookup_host(&endpoint).await {
        Ok(addr_list) => addr_list,
        Err(err) => {
            tracing::debug!(?err);
            return None;
        }
    };

    for addr in addr_list {
        if let Some(con) = con_stack(use_tls, host, con_url, addr).await {
            result_socket = Some(con);
            break;
        }
    }

    let mut socket = match result_socket {
        Some(socket) => socket,
        None => {
            tracing::debug!("failed all sig dns addr connects");
            return None;
        }
    };

    let auth_req = match socket.next().await {
        Some(Ok(auth_req)) => auth_req.into_data(),
        Some(Err(err)) => {
            tracing::debug!(?err);
            return None;
        }
        None => {
            tracing::debug!("InvalidServerAuthReq");
            return None;
        }
    };

    let decode = match wire::Wire::decode(&auth_req) {
        Ok(decode) => decode,
        Err(err) => {
            tracing::debug!(?err);
            return None;
        }
    };

    let (srv_pub, nonce, cipher, got_ice) = match decode {
        wire::Wire::AuthReqV1 {
            srv_pub,
            nonce,
            cipher,
            ice,
        } => (srv_pub, nonce, cipher, ice),
        _ => {
            tracing::debug!("InvalidServerAuthReq");
            return None;
        }
    };

    let con_key = match lair_client
        .crypto_box_xsalsa_open_by_pub_key(
            srv_pub.0.into(),
            x25519_pub.0.into(),
            None,
            nonce.0,
            cipher.0.into(),
        )
        .await
    {
        Ok(con_key) => con_key,
        Err(err) => {
            tracing::debug!(?err);
            return None;
        }
    };

    if let Err(err) = socket
        .send(Message::binary(
            match (wire::Wire::AuthResV1 {
                con_key: match Id::from_slice(&con_key) {
                    Ok(con_key) => con_key,
                    Err(err) => {
                        tracing::debug!(?err);
                        return None;
                    }
                },
                req_addr: true,
            })
            .encode()
            {
                Ok(binary) => binary,
                Err(err) => {
                    tracing::debug!(?err);
                    return None;
                }
            },
        ))
        .await
    {
        tracing::debug!(?err);
        return None;
    }

    tracing::info!(%got_ice, "signal connection established");
    *ice.lock() = Arc::new(got_ice);

    Some(socket)
}

async fn con_manage_connection(
    socket: Socket,
    recv_cb: RecvCb,
    x25519_pub: Id,
    lair_client: &LairClient,
    write_send: WriteSend,
    write_recv: WriteRecv,
) -> (RecvCb, WriteRecv) {
    let recv_cb = Arc::new(tokio::sync::Mutex::new(recv_cb));
    let write_recv = Arc::new(tokio::sync::Mutex::new(write_recv));

    let recv_cb2 = recv_cb.clone();
    let write_recv2 = write_recv.clone();

    macro_rules! dbg_err {
        ($e:expr) => {
            match $e {
                Err(err) => {
                    tracing::debug!(?err);
                    return;
                }
                Ok(r) => r,
            }
        };
    }

    let (mut write, mut read) = socket.split();

    tokio::select! {
        _ = async move {
            let mut recv_cb = recv_cb2.lock().await;
            while let Some(msg) = read.next().await {
                let msg = dbg_err!(msg);
                if let Message::Pong(_) = &msg {
                    tracing::debug!("ws-pong");
                    continue;
                }
                if let Message::Ping(v) = &msg {
                    tracing::debug!("ws-ping");
                    let (s, r) = tokio::sync::oneshot::channel();
                    let _ = write_send.send((Message::Pong(v.clone()), s)).await;
                    if let Err(err) = r.await {
                        tracing::debug!(?err);
                        return;
                    }
                    continue;
                }
                let msg = msg.into_data();
                match dbg_err!(wire::Wire::decode(&msg)) {
                    wire::Wire::DemoV1 { rem_pub } => {
                        recv_cb(SignalMsg::Demo { rem_pub });
                    }
                    wire::Wire::FwdV1 { rem_pub, nonce, cipher } => {
                        if let Err(err) = decode_fwd(
                            &mut recv_cb,
                            &x25519_pub,
                            lair_client,
                            rem_pub,
                            nonce,
                            cipher,
                        ).await {
                            tracing::warn!(?err, "invalid incoming fwd");

                            // MAYBE - should we squelch rem_pub?
                        }
                    }
                    _ => {
                        tracing::debug!("InvalidClientMsg");
                        return;
                    }
                }
            }
        } => (),

        _ = async move {
            let mut write_recv = write_recv2.lock().await;
            while let Some((msg, resp)) = write_recv.recv().await {
                if let Err(err) = write.send(msg).await.map_err(Error::err) {
                    let _ = resp.send(Err(err.err_clone()));
                    tracing::debug!(?err);
                    return;
                }
                let _ = resp.send(Ok(()));
            }
        } => (),
    };

    (
        Arc::try_unwrap(recv_cb)
            .map_err(|_| ())
            .unwrap()
            .into_inner(),
        Arc::try_unwrap(write_recv)
            .map_err(|_| ())
            .unwrap()
            .into_inner(),
    )
}

async fn decode_fwd(
    recv_cb: &mut RecvCb,
    x25519_pub: &Id,
    lair_client: &LairClient,
    rem_pub: Id,
    nonce: wire::Nonce,
    cipher: wire::Cipher,
) -> Result<()> {
    let msg = lair_client
        .crypto_box_xsalsa_open_by_pub_key(
            rem_pub.0.into(),
            x25519_pub.0.into(),
            None,
            nonce.0,
            cipher.0.into(),
        )
        .await?;

    let msg = wire::FwdInnerV1::decode(&msg)?;

    let now = std::time::SystemTime::UNIX_EPOCH
        .elapsed()
        .unwrap()
        .as_secs_f64()
        * 1000.0;

    let seq = msg.get_seq();

    // five minutes in milliseconds
    const FIVE_MIN_MS: f64 = 1000.0 * 60.0 * 5.0;

    if seq > (now + FIVE_MIN_MS) || seq < (now - FIVE_MIN_MS) {
        tracing::warn!(%now, %seq, "Invalid seq");
        return Ok(());
    }

    // TODO - track seq per rem_pub

    match msg {
        wire::FwdInnerV1::Offer { offer, .. } => {
            recv_cb(SignalMsg::Offer { rem_pub, offer });
        }
        wire::FwdInnerV1::Answer { answer, .. } => {
            recv_cb(SignalMsg::Answer { rem_pub, answer });
        }
        wire::FwdInnerV1::Ice { ice, .. } => {
            recv_cb(SignalMsg::Ice { rem_pub, ice });
        }
    }

    Ok(())
}
