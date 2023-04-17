#![deny(missing_docs)]
#![deny(unsafe_code)]
#![deny(warnings)]
#![doc = tx5_core::__doc_header!()]
//! # tx5-discover
//!
//! Holochain WebRTC p2p communication ecosystem lan discovery.

use std::sync::{Arc, Weak};
use parking_lot::Mutex;

/// Re-exported dependencies.
pub mod deps {
    pub use tx5_core::deps::*;
}

pub use tx5_core::{Error, ErrorExt, Id, Result};

const BUF_SIZE: usize = 4096;

/// The default tx5-discover announcement port.
pub const PORT: u16 = 13131;

/// The default tx5-discover ipv4 multicast address.
pub const MULTICAST_V4: std::net::Ipv4Addr = std::net::Ipv4Addr::new(233, 252, 252, 252);

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

struct Tx5DiscoverInner {
    v4_task: tokio::task::JoinHandle<()>,
    config: Tx5DiscoverConfig,
    v4_announce: Mutex<Option<Arc<tokio::net::UdpSocket>>>,
}

impl Drop for Tx5DiscoverInner {
    fn drop(&mut self) {
        self.v4_task.abort();
    }
}

/// Discovery of LAN tx5 peers.
#[derive(Clone)]
pub struct Tx5Discover(Arc<Tx5DiscoverInner>);

impl Tx5Discover {
    /// Create a new tx5-discovery instance.
    pub fn new(config: Tx5DiscoverConfig) -> Self {
        let (s, r) = tokio::sync::oneshot::channel();
        let v4_task = tokio::task::spawn(v4_task(r));
        let inner = Arc::new(Tx5DiscoverInner {
            v4_task,
            config,
            v4_announce: Mutex::new(None),
        });
        let _ = s.send(Arc::downgrade(&inner));
        Self(inner)
    }
}

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
                    tokio::time::sleep(std::time::Duration::from_secs(20)).await;
                    continue;
                }
                Ok(v4_announce) => {
                    *inner.v4_announce.lock() = Some(v4_announce.clone());
                    let addr = std::net::SocketAddrV4::new(inner.config.multi_v4_addr, inner.config.port);
                    if let Err(err) = v4_announce.send_to(b"test-announce", addr).await {
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

async fn bind_v4_announce(inner: &Tx5DiscoverInner) -> Result<Arc<tokio::net::UdpSocket>> {
    let v4_addr = inner.config.multi_v4_addr;
    let port = inner.config.port;
    Ok(Arc::new(tokio::task::spawn_blocking(move || {
        let s = socket2::Socket::new(
            socket2::Domain::IPV4,
            socket2::Type::DGRAM,
            Some(socket2::Protocol::UDP),
        )?;
        s.join_multicast_v4(
            &v4_addr,
            &std::net::Ipv4Addr::UNSPECIFIED,
        )?;
        s.set_multicast_loop_v4(true)?;
        s.set_nonblocking(true)?;
        s.set_reuse_address(true)?;
        s.set_ttl(32)?; // site

        let bind_addr = std::net::SocketAddrV4::new([0, 0, 0, 0].into(), port);
        s.bind(&bind_addr.into())?;

        tokio::net::UdpSocket::from_std(s.into())
    }).await??))
}

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

        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
}
