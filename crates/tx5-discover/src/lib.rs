#![deny(missing_docs)]
#![deny(unsafe_code)]
#![deny(warnings)]
#![doc = tx5_core::__doc_header!()]
//! # tx5-discover
//!
//! Holochain WebRTC p2p communication ecosystem lan discovery.

use std::future::Future;
use std::sync::{Arc, Weak};
use parking_lot::Mutex;
use sodoken::crypto_box::curve25519xsalsa20poly1305 as crypto_box;

/// Re-exported dependencies.
pub mod deps {
    pub use tx5_core::deps::*;
}

pub use tx5_core::{Error, ErrorExt, Id, Result};

/// The default tx5-discover announcement port.
pub const PORT: u16 = 13131;

/// The default tx5-discover ipv4 multicast address.
pub const MULTICAST_V4: std::net::Ipv4Addr =
    std::net::Ipv4Addr::new(233, 252, 252, 252);

/// The default tx5-discover ipv6 multicast address.
pub const MULTICAST_V6: std::net::Ipv6Addr =
    std::net::Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0, 0, 0xacac);

/// Raw callback type for receiving from a udp socket.
pub type RawRecv = Arc<
    dyn Fn(Result<(Weak<tokio::net::UdpSocket>, Vec<u8>, std::net::SocketAddr)>) + 'static + Send + Sync,
>;

/// Raw udp socket wrapper.
pub struct Socket {
    socket: Arc<tokio::net::UdpSocket>,
    recv_task: tokio::task::JoinHandle<()>,
}

impl Drop for Socket {
    fn drop(&mut self) {
        self.recv_task.abort();
    }
}

impl Socket {
    /// Bind a new ipv4 udp socket.
    pub async fn with_v4(
        iface: std::net::Ipv4Addr,
        mcast: Option<std::net::Ipv4Addr>,
        port: u16,
        raw_recv: RawRecv,
    ) -> Result<Arc<Self>> {
        let s = socket2::Socket::new(
            socket2::Domain::IPV4,
            socket2::Type::DGRAM,
            Some(socket2::Protocol::UDP),
        )?;

        s.set_nonblocking(true)?;

        if let Some(mcast) = mcast {
            s.set_reuse_address(true)?;
            #[cfg(unix)]
            s.set_reuse_port(true)?;
            s.join_multicast_v4(&mcast, &std::net::Ipv4Addr::UNSPECIFIED)?;
            s.set_multicast_loop_v4(true)?;
            s.set_ttl(32)?; // site
        }

        let bind_addr = std::net::SocketAddrV4::new(iface, port);
        s.bind(&bind_addr.into())?;

        let socket = Arc::new(tokio::net::UdpSocket::from_std(s.into())?);

        let recv_sock = Arc::downgrade(&socket);
        let recv_task = tokio::task::spawn(async move {
            let mut buf = [0; 4096];
            loop {
                let socket = match recv_sock.upgrade() {
                    None => break,
                    Some(recv_sock) => recv_sock,
                };

                match socket.recv_from(&mut buf).await {
                    Ok((len, addr)) => {
                        let data = buf[..len].to_vec();
                        raw_recv(Ok((recv_sock.clone(), data, addr)));
                    }
                    Err(err) => raw_recv(Err(err)),
                }
            }
        });

        Ok(Arc::new(Self { socket, recv_task }))
    }

    /// Bind a new ipv6 udp socket.
    pub async fn with_v6(
        iface: std::net::Ipv6Addr,
        mcast: Option<(std::net::Ipv6Addr, u32)>,
        port: u16,
        raw_recv: RawRecv,
    ) -> Result<Arc<Self>> {
        let s = socket2::Socket::new(
            socket2::Domain::IPV6,
            socket2::Type::DGRAM,
            Some(socket2::Protocol::UDP),
        )?;

        s.set_nonblocking(true)?;

        if let Some((mcast, index)) = mcast {
            s.set_reuse_address(true)?;
            #[cfg(unix)]
            s.set_reuse_port(true)?;
            s.set_only_v6(true)?;
            assert!(mcast.is_multicast());
            s.join_multicast_v6(&mcast, index)?;
            s.set_multicast_loop_v6(true)?;
            //s.set_multicast_hops_v6(1)?;
        }

        let bind_addr = std::net::SocketAddrV6::new(iface, port, 0, 0);
        s.bind(&bind_addr.into())?;

        let socket = Arc::new(tokio::net::UdpSocket::from_std(s.into())?);

        let recv_sock = Arc::downgrade(&socket);
        let recv_task = tokio::task::spawn(async move {
            let mut buf = [0; 4096];
            loop {
                let socket = match recv_sock.upgrade() {
                    None => break,
                    Some(recv_sock) => recv_sock,
                };

                match socket.recv_from(&mut buf).await {
                    Ok((len, addr)) => {
                        let data = buf[..len].to_vec();
                        raw_recv(Ok((recv_sock.clone(), data, addr)));
                    }
                    Err(err) => raw_recv(Err(err)),
                }
            }
        });

        Ok(Arc::new(Self { socket, recv_task }))
    }

    /// Send data from our bound udp socket.
    pub fn send(
        &self,
        data: Vec<u8>,
        addr: std::net::SocketAddr,
    ) -> impl Future<Output = Result<()>> + 'static + Send {
        let socket = self.socket.clone();
        async move { socket.send_to(&data, addr).await.map(|_| ()) }
    }
}

/// Let's just bind to everything and anything as well.
pub struct Shotgun {
    _v4_listeners: Vec<Arc<Socket>>,
    _v6_listeners: Vec<Arc<Socket>>,
    v4_senders: Vec<Arc<Socket>>,
    v6_senders: Vec<Arc<Socket>>,
    port: u16,
    mcast_v4: std::net::Ipv4Addr,
    mcast_v6: std::net::Ipv6Addr,
}

impl Shotgun {
    /// Construct a new shotgun set of udp sockets.
    pub async fn new(
        raw_recv: RawRecv,
        port: u16,
        mcast_v4: std::net::Ipv4Addr,
        mcast_v6: std::net::Ipv6Addr,
    ) -> Result<Arc<Shotgun>> {
        let mut errors = Vec::new();
        let mut _v4_listeners = Vec::new();
        let mut _v6_listeners = Vec::new();
        let mut v4_senders = Vec::new();
        let mut v6_senders = Vec::new();

        macro_rules! bind_v4 {
            ($addr:expr) => {{
                match Socket::with_v4(
                    $addr,
                    Some(mcast_v4),
                    port,
                    raw_recv.clone(),
                )
                .await
                {
                    Ok(socket) => _v4_listeners.push(socket),
                    Err(err) => {
                        tracing::debug!(?err);
                        errors.push(err);
                    }
                }
                match Socket::with_v4($addr, None, 0, raw_recv.clone()).await {
                    Ok(socket) => v4_senders.push(socket),
                    Err(err) => {
                        tracing::debug!(?err);
                        errors.push(err);
                    }
                }
            }};
        }

        macro_rules! bind_v6 {
            ($addr:expr, $idx:expr) => {{
                match Socket::with_v6(
                    $addr,
                    Some((mcast_v6, $idx)),
                    port,
                    raw_recv.clone(),
                )
                .await
                {
                    Ok(socket) => _v6_listeners.push(socket),
                    Err(err) => {
                        tracing::debug!(?err);
                        errors.push(err);
                    }
                }
                match Socket::with_v6($addr, None, 0, raw_recv.clone()).await {
                    Ok(socket) => v6_senders.push(socket),
                    Err(err) => {
                        tracing::debug!(?err);
                        errors.push(err);
                    }
                }
            }};
        }

        bind_v4!(std::net::Ipv4Addr::UNSPECIFIED);
        bind_v6!(std::net::Ipv6Addr::UNSPECIFIED, 0);

        if let Ok(iface_list) = if_addrs::get_if_addrs() {
            for iface in iface_list {
                let ip = iface.ip();

                if ip.is_unspecified() {
                    continue;
                }

                if ip.is_loopback() {
                    continue;
                }

                let index = iface.index.unwrap_or_default();

                tracing::info!(?ip, %index, "BINDING");

                match ip {
                    std::net::IpAddr::V4(ip) => bind_v4!(ip),
                    std::net::IpAddr::V6(ip) => bind_v6!(ip, index),
                }
            }
        }

        if _v4_listeners.is_empty() && _v6_listeners.is_empty() {
            return Err(Error::str(format!(
                "could not bind listener: {errors:?}",
            )));
        }

        if v4_senders.is_empty() && v6_senders.is_empty() {
            return Err(Error::str(format!(
                "could not bind sender: {errors:?}",
            )));
        }

        Ok(Arc::new(Self {
            _v4_listeners,
            _v6_listeners,
            v4_senders,
            v6_senders,
            port,
            mcast_v4,
            mcast_v6,
        }))
    }

    /// Send a multicast announcement.
    pub fn multicast(
        &self,
        data: Vec<u8>,
    ) -> impl Future<Output = Result<()>> + 'static + Send {
        let v4 = self
            .v4_senders
            .iter()
            .map(|s| s.socket.clone())
            .collect::<Vec<_>>();
        let v6 = self
            .v6_senders
            .iter()
            .map(|s| s.socket.clone())
            .collect::<Vec<_>>();
        let port = self.port;
        let mcast_v4 = self.mcast_v4;
        let mcast_v6 = self.mcast_v6;
        async move {
            let mut errors = Vec::new();
            let mut success = false;

            let addr = std::net::SocketAddrV4::new(mcast_v4, port);

            for s in v4 {
                match s.send_to(&data, addr).await {
                    Ok(_) => success = true,
                    Err(err) => {
                        tracing::debug!(?err);
                        errors.push(err);
                    }
                }
            }

            let addr = std::net::SocketAddrV6::new(mcast_v6, port, 0, 0);

            for s in v6 {
                match s.send_to(&data, addr).await {
                    Ok(_) => success = true,
                    Err(err) => {
                        tracing::debug!(?err);
                        errors.push(err);
                    }
                }
            }

            if !success {
                return Err(Error::str(format!("failed to send: {errors:?}",)));
            }

            Ok(())
        }
    }
}

/// A handle for communicating to a LAN peer.
#[derive(Clone)]
pub struct Peer(Weak<tokio::net::UdpSocket>, std::net::SocketAddr);

/// Discovery events.
pub enum Discovery {
    /// A peer exists on the local network.
    Peer {
        /// A handle for communicating to the local network peer.
        peer: Peer,

        /// Application data the peer is making available.
        data: Vec<Vec<u8>>,
    },

    /// An incoming WebRTC Offer from the local network peer.
    Offer {
        /// A handle for communicating to the local network peer.
        peer: Peer,

        /// The WebRTC offer data.
        data: Vec<u8>,
    },

    /// An incoming WebRTC Answer from the local network peer.
    Answer {
        /// A handle for communicating to the local network peer.
        peer: Peer,

        /// The WebRTC answer data.
        data: Vec<u8>,
    },

    /// An incoming WebRTC ICE message from the local network peer.
    ICE {
        /// A handle for communicating to the local network peer.
        peer: Peer,

        /// The WebRTC ice data.
        data: Vec<u8>,
    },
}

/// Callback type for incoming discovery.
pub type RecvCb = Arc<
    dyn Fn(Result<Discovery>) + 'static + Send + Sync,
>;

/// Configure a tx5-discover instance.
pub struct Tx5DiscoverConfig {
    /// The announcement port.
    pub port: u16,

    /// The ipv4 multicast address.
    pub multi_v4_addr: std::net::Ipv4Addr,

    /// The ipv6 multicast address.
    pub multi_v6_addr: std::net::Ipv6Addr,

    /// The callback for discovery events.
    pub recv_cb: RecvCb,
}

impl Default for Tx5DiscoverConfig {
    fn default() -> Self {
        Self {
            port: PORT,
            multi_v4_addr: MULTICAST_V4,
            multi_v6_addr: MULTICAST_V6,
            recv_cb: Arc::new(|_| ()),
        }
    }
}

impl Tx5DiscoverConfig {
    /// Set the discovery event receiver.
    pub fn with_recv_cb<Cb>(mut self, cb: Cb) -> Self
    where
        Cb: Fn(Result<Discovery>) + 'static + Send + Sync,
    {
        self.recv_cb = Arc::new(cb);
        self
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct Pubkey([u8; 32]);

impl std::fmt::Debug for Pubkey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = self.to_base64();
        f.write_str(&s)
    }
}

impl From<sodoken::BufWriteSized<32>> for Pubkey {
    fn from(f: sodoken::BufWriteSized<32>) -> Self {
        Self(*f.read_lock_sized())
    }
}

impl Pubkey {
    fn to_base64(&self) -> String {
        base64::encode_config(self.0, base64::URL_SAFE_NO_PAD)
    }
}

/// Tx5 local network discovery instance.
pub struct Tx5Discover {
    discover_task: tokio::task::JoinHandle<()>,
    pubkey: Pubkey,
    //config: Arc<Tx5DiscoverConfig>,
}

impl Drop for Tx5Discover {
    fn drop(&mut self) {
        self.discover_task.abort();
    }
}

impl Tx5Discover {
    /// Construct a new tx5 local network discovery instance.
    pub async fn new(config: Tx5DiscoverConfig) -> Result<Self> {
        let config = Arc::new(config);

        let pubkey = sodoken::BufWriteSized::new_no_lock();
        let seckey = sodoken::BufWriteSized::new_mem_locked()?;
        crypto_box::keypair(pubkey.clone(), seckey.clone()).await?;

        let pubkey = Pubkey::from(pubkey);
        let seckey = Arc::new(seckey.to_read_sized());

        let discover_task = tokio::task::spawn(discover_task(
            config,
            pubkey,
            seckey,
        ));

        Ok(Self {
            discover_task,
            pubkey,
        })
    }

    /// Get the local network discovery identifier for this instance.
    pub fn id(&self) -> String {
        self.pubkey.to_base64()
    }
}

struct DiscoverState {
    last_announce: std::time::Instant,
    announce_int: std::time::Duration,
    last_recv: std::time::Instant,
    state_counter: u64,
}

impl Default for DiscoverState {
    fn default() -> Self {
        Self {
            last_announce: std::time::Instant::now(),
            announce_int: std::time::Duration::from_secs(15),
            last_recv: std::time::Instant::now(),
            state_counter: 0,
        }
    }
}

enum DiscoverStateAction {
    Idle,
    Abort,
    Announce(Vec<u8>),
}

impl DiscoverState {
    fn recv(&mut self, _res: Result<(Weak<tokio::net::UdpSocket>, Vec<u8>, std::net::SocketAddr)>) {
        self.last_recv = std::time::Instant::now();
        todo!()
    }

    fn get_action(
        &mut self,
        pubkey: &Pubkey,
    ) -> DiscoverStateAction {
        // if we haven't received anything in the last 30 seconds
        // we might need to re-bind our sockets. Abort for now.
        if self.last_recv.elapsed() >= std::time::Duration::from_secs(30) {
            return DiscoverStateAction::Abort;
        }

        if self.last_announce.elapsed() >= self.announce_int {
            return DiscoverStateAction::Announce(self.mark_announce(pubkey));
        }


        DiscoverStateAction::Idle
    }

    fn mark_announce(
        &mut self,
        pubkey: &Pubkey,
    ) -> Vec<u8> {
        use rand::Rng;
        self.last_announce = std::time::Instant::now();
        self.announce_int = std::time::Duration::from_millis(rand::thread_rng().gen_range(10000..20000));
        let mut out = Vec::with_capacity(4 + 32 + 8);
        out.extend_from_slice(b"0tx5");
        out.extend_from_slice(&pubkey.0[..]);
        out.extend_from_slice(&self.state_counter.to_le_bytes());
        out
    }
}

async fn discover_task(
    config: Arc<Tx5DiscoverConfig>,
    pubkey: Pubkey,
    seckey: Arc<sodoken::BufReadSized<32>>,
) {
    let state = Arc::new(Mutex::new(DiscoverState::default()));

    loop {
        let state_recv = state.clone();
        if let Ok(shotgun) = Shotgun::new(
            Arc::new(move |res| {
                state_recv.lock().recv(res);
            }),
            config.port,
            config.multi_v4_addr,
            config.multi_v6_addr,
        ).await {
            discover_task_2(
                config.clone(),
                state.clone(),
                shotgun,
                pubkey,
                seckey.clone(),
            ).await;
        }

        // wait some time between binding attempts
        tokio::time::sleep(std::time::Duration::from_secs(20)).await;
    }
}

async fn discover_task_2(
    _config: Arc<Tx5DiscoverConfig>,
    state: Arc<Mutex<DiscoverState>>,
    shotgun: Arc<Shotgun>,
    pubkey: Pubkey,
    _seckey: Arc<sodoken::BufReadSized<32>>,
) {
    // always announce right away
    let announce = state.lock().mark_announce(&pubkey);
    if let Err(err) = shotgun.multicast(announce).await {
        tracing::debug!(?err);
        return;
    }

    loop {
        let mut did_something = false;

        let action = state.lock().get_action(&pubkey);
        match action {
            DiscoverStateAction::Idle => (),
            DiscoverStateAction::Abort => return,
            DiscoverStateAction::Announce(announce) => {
                did_something = true;
                if let Err(err) = shotgun.multicast(announce).await {
                    tracing::debug!(?err);
                    return;
                }
            }
        }

        if !did_something {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    }
}

#[cfg(test)]
mod test;
