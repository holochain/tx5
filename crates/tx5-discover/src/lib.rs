#![deny(missing_docs)]
#![deny(unsafe_code)]
#![deny(warnings)]
#![doc = tx5_core::__doc_header!()]
//! # tx5-discover
//!
//! Holochain WebRTC p2p communication ecosystem lan discovery.

use std::future::Future;
use std::sync::Arc;

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
    dyn Fn(Result<(Vec<u8>, std::net::SocketAddr)>) + 'static + Send + Sync,
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

        s.set_reuse_address(true)?;
        s.set_nonblocking(true)?;

        if let Some(mcast) = mcast {
            s.join_multicast_v4(&mcast, &std::net::Ipv4Addr::UNSPECIFIED)?;
            s.set_multicast_loop_v4(true)?;
            s.set_ttl(32)?; // site
        }

        let bind_addr = std::net::SocketAddrV4::new(iface, port);
        s.bind(&bind_addr.into())?;

        let socket = Arc::new(tokio::net::UdpSocket::from_std(s.into())?);

        let recv_sock = socket.clone();
        let recv_task = tokio::task::spawn(async move {
            let mut buf = [0; 4096];
            loop {
                match recv_sock.recv_from(&mut buf).await {
                    Ok((len, addr)) => {
                        let data = buf[..len].to_vec();
                        raw_recv(Ok((data, addr)));
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

        s.set_reuse_address(true)?;
        s.set_nonblocking(true)?;

        if let Some((mcast, index)) = mcast {
            s.set_only_v6(true)?;
            assert!(mcast.is_multicast());
            s.join_multicast_v6(&mcast, index)?;
            s.set_multicast_loop_v6(true)?;
            //s.set_multicast_hops_v6(1)?;
        }

        let bind_addr = std::net::SocketAddrV6::new(iface, port, 0, 0);
        s.bind(&bind_addr.into())?;

        let socket = Arc::new(tokio::net::UdpSocket::from_std(s.into())?);

        let recv_sock = socket.clone();
        let recv_task = tokio::task::spawn(async move {
            let mut buf = [0; 4096];
            loop {
                match recv_sock.recv_from(&mut buf).await {
                    Ok((len, addr)) => {
                        let data = buf[..len].to_vec();
                        raw_recv(Ok((data, addr)));
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

                println!("BINDING TO ip: {ip:?} index: {index}");

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

/*
use parking_lot::Mutex;
use std::sync::{Arc, Weak};
//use std::future::Future;
use std::collections::HashMap;

/// Re-exported dependencies.
pub mod deps {
    pub use tx5_core::deps::*;
}

pub use tx5_core::{Error, ErrorExt, Id, Result};

const BUF_SIZE: usize = 4096;

/// The default tx5-discover announcement port.
pub const PORT: u16 = 13131;

/// The default tx5-discover ipv4 multicast address.
pub const MULTICAST_V4: std::net::Ipv4Addr =
    std::net::Ipv4Addr::new(233, 252, 252, 252);

/// Configure a tx5-discover instance.
pub struct Tx5DiscoverConfig {
    /// The announcement port.
    pub port: u16,

    /// The ipv4 multicast address.
    pub multi_v4_addr: std::net::Ipv4Addr,
}

impl Default for Tx5DiscoverConfig {
    fn default() -> Self {
        Self {
            port: PORT,
            multi_v4_addr: MULTICAST_V4,
        }
    }
}

struct Socket {
    rcv_task: tokio::task::JoinHandle<()>,
    socket: Arc<tokio::net::UdpSocket>,
}

impl Drop for Socket {
    fn drop(&mut self) {
        self.rcv_task.abort();
    }
}

impl Socket {
    async fn with_v4(
        iface: std::net::Ipv4Addr,
        mcast: std::net::Ipv4Addr,
        port: u16,
    ) -> Result<Arc<Self>> {
        let socket = Arc::new(tokio::task::spawn_blocking(move || {
            let s = socket2::Socket::new(
                socket2::Domain::IPV4,
                socket2::Type::DGRAM,
                Some(socket2::Protocol::UDP),
            )?;
            s.join_multicast_v4(&mcast, &iface)?;
            s.set_multicast_loop_v4(true)?;
            s.set_nonblocking(true)?;
            s.set_reuse_address(true)?;
            s.set_ttl(32)?; // site

            let bind_addr = std::net::SocketAddrV4::new(iface, port);
            s.bind(&bind_addr.into())?;

            tokio::net::UdpSocket::from_std(s.into())
        })
        .await??);

        let rcv_task = tokio::task::spawn(rcv_task(socket.clone()));

        Ok(Arc::new(Self {
            rcv_task,
            socket,
        }))
    }
}

type SocketMap = HashMap<std::net::IpAddr, Arc<Socket>>;

struct Tx5DiscoverInner {
    iface_task: tokio::task::JoinHandle<()>,
    announce_task: tokio::task::JoinHandle<()>,
    config: Tx5DiscoverConfig,
    sockets: Mutex<SocketMap>,
}

impl Drop for Tx5DiscoverInner {
    fn drop(&mut self) {
        self.iface_task.abort();
        self.announce_task.abort();
    }
}

/// Discovery of LAN tx5 peers.
#[derive(Clone)]
pub struct Tx5Discover(Arc<Tx5DiscoverInner>);

impl Tx5Discover {
    /// Create a new tx5-discovery instance.
    pub fn new(config: Tx5DiscoverConfig) -> Self {
        let (iface_s, iface_r) = tokio::sync::oneshot::channel();
        let iface_task = tokio::task::spawn(iface_task(iface_r));

        let (announce_s, announce_r) = tokio::sync::oneshot::channel();
        let announce_task = tokio::task::spawn(announce_task(announce_r));

        let inner = Arc::new(Tx5DiscoverInner {
            iface_task,
            announce_task,
            config,
            sockets: Mutex::new(HashMap::new()),
        });

        let _ = iface_s.send(Arc::downgrade(&inner));
        let _ = announce_s.send(Arc::downgrade(&inner));

        Self(inner)
    }
}

async fn iface_task(r: tokio::sync::oneshot::Receiver<Weak<Tx5DiscoverInner>>) {
    let weak = match r.await {
        Err(err) => {
            tracing::error!(?err);
            return;
        }
        Ok(weak) => weak,
    };

    loop {
        let inner = match weak.upgrade() {
            None => return,
            Some(inner) => inner,
        };

        let mut addrs = Vec::new();
        if let Ok(iface_list) = if_addrs::get_if_addrs() {
            for iface in iface_list {
                let ip = iface.ip();

                if ip.is_unspecified() {
                    continue;
                }

                if ip.is_loopback() {
                    continue;
                }

                addrs.push(ip);
            }
        }

        for addr in addrs {
            if inner.sockets.lock().contains_key(&addr) {
                continue;
            }

            match addr {
                std::net::IpAddr::V4(addr) => {
                    let socket = match Socket::with_v4(
                        addr,
                        inner.config.multi_v4_addr,
                        inner.config.port,
                    ).await {
                        Err(err) => {
                            tracing::warn!(?err);
                            continue;
                        }
                        Ok(socket) => socket,
                    };
                    inner.sockets.lock().insert(addr.into(), socket);
                    tracing::info!("bound {addr:?}");
                }
                std::net::IpAddr::V6(_addr) => {
                }
            }
        }

        tokio::time::sleep(std::time::Duration::from_secs(20)).await;
    }
}

async fn announce_task(r: tokio::sync::oneshot::Receiver<Weak<Tx5DiscoverInner>>) {
    let weak = match r.await {
        Err(err) => {
            tracing::error!(?err);
            return;
        }
        Ok(weak) => weak,
    };

    loop {
        let s = 5; // todo - randomize
        tokio::time::sleep(std::time::Duration::from_secs(s)).await;

        let inner = match weak.upgrade() {
            None => return,
            Some(inner) => inner,
        };

        let addr = std::net::SocketAddr::new(inner.config.multi_v4_addr.into(), inner.config.port);

        let socket_list = inner.sockets.lock().values().cloned().collect::<Vec<_>>();
        for socket in socket_list {
            if let Err(err) = socket.socket.send_to(b"test-announce", addr).await {
                tracing::warn!(?err);
            } else {
                tracing::trace!("sent");
            }
        }
    }
}

async fn rcv_task(socket: Arc<tokio::net::UdpSocket>) {
    let mut data = [0; BUF_SIZE];

    loop {
        let (len, addr) = match socket.recv_from(&mut data).await {
            Err(err) => {
                tracing::warn!(?err);
                if true {
                    todo!("remove it from socket map");
                }
                break;
            }
            Ok(r) => r,
        };
        let s = String::from_utf8_lossy(&data[..len]);
        tracing::info!(%s, ?addr, "got");
    }
}

/*
async fn v4_task(r: tokio::sync::oneshot::Receiver<Weak<Tx5DiscoverInner>>) {
    let weak = match r.await {
        Err(err) => {
            tracing::error!(?err);
            return;
        }
        Ok(weak) => weak,
    };

    let mut buf = [0; BUF_SIZE];

    loop {
        let inner = match weak.upgrade() {
            None => return,
            Some(inner) => inner,
        };

        let v4_announce = inner.v4_announce.lock().clone();
        let v4_announce = if let Some(v4_announce) = v4_announce {
            v4_announce.clone()
        } else {
            match bind_v4_announce(&inner).await {
                Err(err) => {
                    tracing::warn!(?err);
                    tokio::time::sleep(std::time::Duration::from_secs(20))
                        .await;
                    continue;
                }
                Ok(v4_announce) => {
                    *inner.v4_announce.lock() = Some(v4_announce.clone());
                    let addr = std::net::SocketAddrV4::new(
                        inner.config.multi_v4_addr,
                        inner.config.port,
                    );
                    if let Err(err) =
                        v4_announce.send_to(b"test-announce", addr).await
                    {
                        tracing::warn!(?err);
                    }
                    v4_announce
                }
            }
        };

        match v4_announce.recv_from(&mut buf).await {
            Err(err) => {
                tracing::warn!(?err);
                *inner.v4_announce.lock() = None;
            }
            Ok((len, addr)) => {
                let s = String::from_utf8_lossy(&buf[..len]);
                tracing::info!(%s, ?addr);
            }
        }
    }
}

async fn bind_v4_announce(
    inner: &Tx5DiscoverInner,
) -> Result<Arc<tokio::net::UdpSocket>> {
    let v4_addr = inner.config.multi_v4_addr;
    let port = inner.config.port;
    Ok(Arc::new(
        tokio::task::spawn_blocking(move || {
            let s = socket2::Socket::new(
                socket2::Domain::IPV4,
                socket2::Type::DGRAM,
                Some(socket2::Protocol::UDP),
            )?;
            s.join_multicast_v4(&v4_addr, &std::net::Ipv4Addr::UNSPECIFIED)?;
            s.set_multicast_loop_v4(true)?;
            s.set_nonblocking(true)?;
            s.set_reuse_address(true)?;
            s.set_ttl(32)?; // site

            let bind_addr =
                std::net::SocketAddrV4::new([0, 0, 0, 0].into(), port);
            s.bind(&bind_addr.into())?;

            tokio::net::UdpSocket::from_std(s.into())
        })
        .await??,
    ))
}
*/

#[cfg(test)]
mod test {
    use super::*;

    fn init_tracing() {
        let subscriber = tracing_subscriber::FmtSubscriber::builder()
            .with_env_filter(
                tracing_subscriber::filter::EnvFilter::from_default_env(),
            )
            .with_file(true)
            .with_line_number(true)
            .finish();
        let _ = tracing::subscriber::set_global_default(subscriber);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sanity() {
        init_tracing();

        let _d = Tx5Discover::new(Default::default());

        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
    }
}
*/

#[cfg(test)]
mod test;
