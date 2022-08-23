//! Server-side connection types.

// srv coms
//
// -> cli opens wss connection to:
//   wss://<srv_host>:<srv_port>/<cli_x25519_pub>
//
// <- srv AUTH as crypto box to cli_x25519_pub with con_key
//
// -> cli AUTH reg must include valid con_key, reg bool within x timeout
//
// -> cli FWD fwd to target
//
// <- srv FWD sent if cli is correctly reg'd

use crate::*;
use futures::sink::SinkExt;
use futures::stream::StreamExt;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use tokio_tungstenite::tungstenite::Message;

type Socket = tokio_tungstenite::WebSocketStream<
    tokio_rustls::TlsStream<tokio::net::TcpStream>,
>;

/// Builder for constructing a Srv instance.
#[derive(Debug)]
pub struct SrvBuilder {
    port: u16,
    tls: Option<tls::TlsConfig>,
    ice_servers: String,
    allow_demo: bool,
}

impl Default for SrvBuilder {
    fn default() -> Self {
        Self {
            port: 8443,
            tls: None,
            ice_servers: "[]".to_string(),
            allow_demo: false,
        }
    }
}

impl SrvBuilder {
    /// Set the port to bind.
    pub fn set_port(&mut self, port: u16) {
        self.port = port;
    }

    /// Apply a port to bind.
    pub fn with_port(mut self, port: u16) -> Self {
        self.set_port(port);
        self
    }

    /// Set the TlsConfig.
    pub fn set_tls(&mut self, tls: tls::TlsConfig) {
        self.tls = Some(tls);
    }

    /// Apply a TlsConfig.
    pub fn with_tls(mut self, tls: tls::TlsConfig) -> Self {
        self.set_tls(tls);
        self
    }

    /// Set the ice servers to publish.
    pub fn set_ice_servers(&mut self, ice_servers: String) {
        self.ice_servers = ice_servers;
    }

    /// Apply ice servers to publish.
    pub fn with_ice_servers(mut self, ice_servers: String) -> Self {
        self.set_ice_servers(ice_servers);
        self
    }

    /// Set the allow_demo flag.
    pub fn set_allow_demo(&mut self, allow_demo: bool) {
        self.allow_demo = allow_demo;
    }

    /// Apply the allow_demo flag.
    pub fn with_allow_demo(mut self, allow_demo: bool) -> Self {
        self.set_allow_demo(allow_demo);
        self
    }

    /// Build the Srv instance.
    pub async fn build(self) -> Result<Srv> {
        Srv::priv_build(self).await
    }
}

/// Server-side connection type.
pub struct Srv {
    srv_term: util::Term,
    bound_port: u16,
}

impl Drop for Srv {
    fn drop(&mut self) {
        self.srv_term.term();
    }
}

impl Srv {
    /// Get a SrvBuilder.
    pub fn builder() -> SrvBuilder {
        SrvBuilder::default()
    }

    /// Shutdown this server instance.
    pub fn close(&self) {
        self.srv_term.term();
    }

    /// Get the port that was bound.
    pub fn bound_port(&self) -> u16 {
        self.bound_port
    }

    // -- private -- //

    async fn priv_build(builder: SrvBuilder) -> Result<Self> {
        tracing::info!(config=?builder, "start server");

        let SrvBuilder {
            port,
            tls,
            ice_servers,
            allow_demo,
        } = builder;
        let ice: Arc<[u8]> = ice_servers.into_bytes().into();

        let tls = match tls {
            Some(tls) => tls,
            None => return Err(Error::id("TlsRequired")),
        };

        let srv_term = util::Term::new("srv_term", None);

        let con_map = ConMap::new();

        let ip_limit = IpLimit::new();

        let listener = tokio::task::spawn_blocking(move || {
            let socket = socket2::Socket::new(
                socket2::Domain::IPV6,
                socket2::Type::STREAM,
                None,
            )?;
            {
                // not safe on windows, allows port-jacking
                #[cfg(not(windows))]
                socket.set_reuse_address(true)?;
            }
            socket.set_only_v6(false)?;
            socket.set_nonblocking(true)?;
            let address = std::net::SocketAddr::new(
                std::net::Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 0).into(),
                port,
            );
            socket.bind(&address.into())?;
            socket.listen(128)?;
            tokio::net::TcpListener::from_std(socket.into())
        })
        .await??;

        let bound_port = listener.local_addr()?.port();

        srv_term.spawn_err(
            listener_task(
                tls.clone(),
                srv_term.clone(),
                listener,
                ice.clone(),
                ip_limit.clone(),
                con_map.clone(),
                allow_demo,
            ),
            |err| {
                tracing::debug!(?err, "ListenerClosed");
            },
        );

        Ok(Self {
            srv_term,
            bound_port,
        })
    }
}

struct Con {
    con_term: util::Term,
    sink: futures::stream::SplitSink<Socket, Message>,
}

impl Con {
    /// Send data out to the remote client of this connection.
    pub async fn send(&mut self, data: Vec<u8>) -> Result<()> {
        let on_term_fut = self.con_term.on_term();
        tokio::select! {
            _ = on_term_fut => Err(Error::id("ConTerm")),
            r = async {
                match self
                    .sink
                    .send(Message::Binary(data))
                    .await
                    .map_err(Error::err)
                {
                    Ok(r) => Ok(r),
                    Err(e) => {
                        self.con_term.term();
                        Err(e)
                    }
                }
            } => r,
        }
    }
}

async fn listener_task(
    tls: tls::TlsConfig,
    srv_term: util::Term,
    listener: tokio::net::TcpListener,
    ice: Arc<[u8]>,
    ip_limit: IpLimit,
    con_map: ConMap,
    allow_demo: bool,
) -> Result<()> {
    loop {
        let (socket, addr) = match listener.accept().await {
            Ok(r) => r,
            Err(err) => {
                tracing::debug!("AcceptError: {:?}", err);
                continue;
            }
        };

        if !ip_limit.check(addr.ip()) {
            tracing::debug!(ip = ?addr.ip(), "IpLimitReached");
            return Err(Error::id("IpLimitReached"));
        }

        let id: sodoken::BufWriteSized<32> =
            sodoken::BufWriteSized::new_no_lock();
        sodoken::random::bytes_buf(id.clone()).await?;
        let id = id.read_lock().to_vec().into_boxed_slice();

        let (con_hnd_send, con_hnd_recv) =
            tokio::sync::mpsc::unbounded_channel();

        con_map.insert(id.clone(), con_hnd_send);

        let con_term = {
            let con_map = con_map.clone();
            let id = id.clone();
            util::Term::new(
                "con_term",
                Some(Arc::new(move || {
                    con_map.remove(&id);
                })),
            )
        };

        let con_term_err = con_term.clone();
        util::Term::spawn_err2(
            &srv_term,
            &con_term,
            con_task(
                tls.clone(),
                srv_term.clone(),
                con_term.clone(),
                id,
                ice.clone(),
                socket,
                addr.ip(),
                ip_limit.clone(),
                con_map.clone(),
                con_hnd_recv,
                allow_demo,
            ),
            move |err| {
                tracing::debug!("ConClosed: {:?}", err);
                con_term_err.term();
            },
        );
    }
}

#[allow(clippy::too_many_arguments)]
async fn con_task(
    tls: tls::TlsConfig,
    srv_term: util::Term,
    con_term: util::Term,
    id: Box<[u8]>,
    ice: Arc<[u8]>,
    socket: tokio::net::TcpStream,
    ip: std::net::IpAddr,
    ip_limit: IpLimit,
    con_map: ConMap,
    mut con_hnd_recv: tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
    allow_demo: bool,
) -> Result<()> {
    let socket = tcp_configure(socket)?;
    let socket = tokio_rustls::TlsAcceptor::from(tls.srv.clone())
        .accept(socket)
        .await?;
    //eprintln!("sni:{:?}", socket.get_ref().1.sni_hostname());

    let socket: tokio_rustls::TlsStream<tokio::net::TcpStream> = socket.into();
    struct Hdr(tokio::sync::oneshot::Sender<String>);
    use tokio_tungstenite::tungstenite::handshake::server;
    impl server::Callback for Hdr {
        fn on_request(
            self,
            request: &server::Request,
            response: server::Response,
        ) -> std::result::Result<server::Response, server::ErrorResponse>
        {
            let _ = self.0.send(request.uri().to_string());
            Ok(response)
        }
    }
    let (s, r) = tokio::sync::oneshot::channel();
    let hdr = Hdr(s);
    let socket: Socket = tokio_tungstenite::accept_hdr_async_with_config(
        socket,
        hdr,
        Some(WS_CONFIG),
    )
    .await
    .map_err(Error::err)?;
    //eprintln!("alpn:{:?}", socket.get_ref().get_ref().1.alpn_protocol());

    let (sink, stream) = socket.split();

    let mut con = Con {
        con_term: con_term.clone(),
        sink,
    };

    use sodoken::crypto_box::curve25519xsalsa20poly1305 as crypto_box;

    let r = r.await.map_err(|_| Error::id("InvalidWssRequest"))?;
    let mut r_iter = r.split('/');
    if r_iter.next().is_none() {
        return Err(Error::id("InvalidClientPubKey"));
    }
    let r = match r_iter.next() {
        Some(r) => r,
        None => return Err(Error::id("InvalidClientPubKey")),
    };
    let r = base64::decode_config(r.as_bytes(), base64::URL_SAFE_NO_PAD)
        .map_err(Error::err)?;
    if r.len() != crypto_box::PUBLICKEYBYTES {
        return Err(Error::id("InvalidClientPubKey"));
    }

    let con_key = <sodoken::BufWriteSized<32>>::new_no_lock();
    sodoken::random::bytes_buf(con_key.clone()).await?;
    let con_key = con_key.to_read_sized();

    let srv_pubkey = sodoken::BufWriteSized::new_no_lock();
    let srv_seckey = sodoken::BufWriteSized::new_mem_locked()?;
    crypto_box::keypair(srv_pubkey.clone(), srv_seckey.clone()).await?;

    let cli_pubkey = sodoken::BufWriteSized::new_no_lock();
    cli_pubkey.write_lock().copy_from_slice(&r);

    let nonce = sodoken::BufWriteSized::new_no_lock();
    sodoken::random::bytes_buf(nonce.clone()).await?;

    let cipher = crypto_box::easy(
        nonce.clone(),
        con_key.clone(),
        cli_pubkey.clone(),
        srv_seckey,
    ).await?;

    let auth_req = crate::wire::SrvWire::AuthReqV1 {
        srv_pub: (*srv_pubkey.read_lock_sized()).into(),
        nonce: (*nonce.read_lock_sized()).into(),
        cipher: cipher.read_lock().to_vec().into_boxed_slice().into(),
        // TODO - don't re-decode this every time
        ice: serde_json::from_slice(&ice)?,
    }.encode()?;

    con.send(auth_req).await?;

    let con_term_err = con_term.clone();
    util::Term::spawn_err2(
        &srv_term,
        &con_term,
        con_recv_task(stream, id, ip, ip_limit, con_map, allow_demo),
        move |err| {
            tracing::debug!("ConRecvError: {:?}", err);
            con_term_err.term();
        },
    );

    while let Some(data) = con_hnd_recv.recv().await {
        con.send(data).await?;
    }

    // always error on end so our term is called
    Err(Error::id("ConClose"))
}

async fn con_recv_task(
    mut stream: futures::stream::SplitStream<Socket>,
    id: Box<[u8]>,
    ip: IpAddr,
    ip_limit: IpLimit,
    con_map: ConMap,
    allow_demo: bool,
) -> Result<()> {
    //let mut need_auth_res = true;
    while let Some(msg) = stream.next().await {
        if !ip_limit.check(ip) {
            tracing::debug!(?ip, "IpLimitReached");
            return Err(Error::id("IpLimitReached"));
        }
        let mut bin_data: Vec<u8> = match msg.map_err(Error::err)? {
            Message::Text(data) => data.into_bytes(),
            Message::Binary(data) => data,
            Message::Ping(data) => data,
            Message::Pong(data) => data,
            Message::Close(close) => {
                return Err(Error::err(format!("{:?}", close)));
            }
            Message::Frame(_) => return Err(Error::id("RawFrame")),
        };

        /*
        let msg = crate::wire::SrvWire::decode(&bin_data)?;

        if need_auth_res {
            match msg {
                crate::wire::SrvWire::AuthResV1 { con_key, req_addr } => {
                }
                _ => return Err(Error::id("InvalidMsg")),
            }
        } else {
            match msg {
                crate::wire::SrvWire::FwdV1 { rem_pub, data } => {
                    // now replace the id with the source id
                    // so the recipient knows who it came from
                    bin_data[4..36].copy_from_slice(&id);

                    con_map.send(&rem_pub, bin_data)
                }
                _ => return Err(Error::id("InvalidMsg")),
            }
        }
        */

        if bin_data.len() < FORWARD.len() + 32 {
            return Err(Error::id("InvalidMsg"));
        }

        match &bin_data[0..4] {
            DEMO => {
                if !allow_demo {
                    return Err(Error::id("InvalidMsg"));
                }

                let mut out = Vec::with_capacity(DEMO.len() + 32 + 32);
                out.extend_from_slice(DEMO);
                out.extend_from_slice(&id);
                out.extend_from_slice(&bin_data[4..36]);

                con_map.broadcast(out);
            }
            FORWARD => {
                let dest_id = bin_data[4..36].to_vec();

                // now replace the id with the source id
                // so the recipient knows who it came from
                bin_data[4..36].copy_from_slice(&id);

                con_map.send(&dest_id, bin_data);
            }
            _ => {
                return Err(Error::id("InvalidMsg"));
            }
        }
    }

    // always error on end so our term is called
    Err(Error::id("ConClose"))
}

// 100 msgs in 5 seconds is 20 messages per second
// but ok to burst a bit initially
const IP_LIMIT_WND: std::time::Duration = std::time::Duration::from_secs(5);
const IP_LIMIT_CNT: usize = 100;

#[derive(Clone)]
struct IpLimit(Arc<Mutex<HashMap<IpAddr, Vec<std::time::Instant>>>>);

impl IpLimit {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(HashMap::new())))
    }

    /// Mark an incoming message NOW.
    /// Check the total message count against the window,
    /// if we're still safe under the limit, return TRUE.
    /// Otherwise, return FALSE, we are over limit.
    pub fn check(&self, ip: IpAddr) -> bool {
        let mut map = self.0.lock();
        let hit = map.entry(ip).or_default();
        let now = std::time::Instant::now();
        hit.push(now);
        hit.retain(|t| *t + IP_LIMIT_WND > now);
        // disable actually limiting for now
        if hit.len() >= IP_LIMIT_CNT {
            tracing::warn!("IpLimitReached (limit disabled for now)");
        }
        true
    }
}

type ConHnd = tokio::sync::mpsc::UnboundedSender<Vec<u8>>;

#[derive(Clone)]
struct ConMap(Arc<Mutex<HashMap<Box<[u8]>, ConHnd>>>);

impl ConMap {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(HashMap::new())))
    }

    pub fn insert(&self, id: Box<[u8]>, h: ConHnd) {
        self.0.lock().insert(id, h);
    }

    pub fn remove(&self, id: &[u8]) {
        self.0.lock().remove(id);
    }

    pub fn send(&self, id: &[u8], data: Vec<u8>) {
        let mut map = self.0.lock();
        let mut remove = false;
        if let Some(h) = map.get(id) {
            if h.send(data).is_err() {
                remove = true;
            }
        }
        if remove {
            map.remove(id);
        }
    }

    pub fn broadcast(&self, data: Vec<u8>) {
        let mut map = self.0.lock();
        let mut remove = Vec::new();
        for (id, h) in map.iter() {
            if h.send(data.clone()).is_err() {
                remove.push(id.clone());
            }
        }
        for id in remove {
            map.remove(&id);
        }
    }
}
