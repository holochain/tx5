#![deny(missing_docs)]
#![deny(unsafe_code)]
#![deny(warnings)]
#![doc = tx5_core::__doc_header!()]
//! # tx5-discover
//!
//! Holochain WebRTC p2p communication ecosystem lan discovery.

use parking_lot::Mutex;
use sodoken::crypto_box::curve25519xsalsa20poly1305 as crypto_box;
use std::future::Future;
use std::sync::{Arc, Weak};
use std::collections::HashMap;

const MAX_PACKET_SIZE: usize = 1024; // fit easily into a 1200 mtu
const HEADER_OVERHEAD: usize = 4 /* outer */ + 16 /* inner */;
const ENC_OVERHEAD: usize = 16 /* mac */ + 24 /* nonce */;
const DISC_DATA_OVERHEAD: usize = 8 /* state counter */ + 8 /* index */;
const DISC_DATA_MAX: usize =
    MAX_PACKET_SIZE - HEADER_OVERHEAD - ENC_OVERHEAD - DISC_DATA_OVERHEAD;

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
type RawRecv = Arc<
    dyn Fn(Result<(Weak<tokio::net::UdpSocket>, Vec<u8>, std::net::SocketAddr)>)
        + 'static
        + Send
        + Sync,
>;

mod socket;
pub(crate) use socket::*;

mod shotgun;
pub(crate) use shotgun::*;

/// A handle for communicating to a LAN peer.
#[derive(Clone)]
pub struct Peer(Weak<tokio::net::UdpSocket>, std::net::SocketAddr);

impl std::fmt::Debug for Peer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Peer").field("addr", &self.1).finish()
    }
}

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

impl std::fmt::Debug for Discovery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Discovery::Peer { peer, data } => f
                .debug_struct("Discovery::Peer")
                .field("peer", peer)
                .field("data", &data.len())
                .finish(),
            Discovery::Offer { peer, data } => f
                .debug_struct("Discovery::Offer")
                .field("peer", peer)
                .field("data", &data.len())
                .finish(),
            Discovery::Answer { peer, data } => f
                .debug_struct("Discovery::Answer")
                .field("peer", peer)
                .field("data", &data.len())
                .finish(),
            Discovery::ICE { peer, data } => f
                .debug_struct("Discovery::ICE")
                .field("peer", peer)
                .field("data", &data.len())
                .finish(),
        }
    }
}

/// Callback type for incoming discovery.
pub type RecvCb = Arc<dyn Fn(Result<Discovery>) + 'static + Send + Sync>;

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
    /// Set the multicast port to use.
    pub fn with_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

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
struct Pubkey([u8; 32], [u8; 14]);

impl std::fmt::Debug for Pubkey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(std::str::from_utf8(&self.1[..]).unwrap())
    }
}

impl From<[u8; 32]> for Pubkey {
    fn from(f: [u8; 32]) -> Self {
        let mut this = Self(f, [0; 14]);

        let mut s = this.to_base64();
        s.replace_range(6..s.len() - 6, "..");
        this.1.copy_from_slice(s.as_bytes());

        this
    }
}

impl From<&[u8]> for Pubkey {
    fn from(f: &[u8]) -> Self {
        let mut inner = [0; 32];
        inner.copy_from_slice(f);
        inner.into()
    }
}

impl From<sodoken::BufWriteSized<32>> for Pubkey {
    fn from(f: sodoken::BufWriteSized<32>) -> Self {
        (*f.read_lock_sized()).into()
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
    state: Arc<Mutex<DiscoverState>>,
    pubkey: Pubkey,
}

impl Drop for Tx5Discover {
    fn drop(&mut self) {
        self.discover_task.abort();
    }
}

impl Tx5Discover {
    /// Construct a new tx5 local network discovery instance.
    pub async fn new<C>(
        config: C,
        discovery_data: Vec<Vec<u8>>,
    ) -> Result<Self>
    where
        C: Into<Arc<Tx5DiscoverConfig>>,
    {
        Self::check_discovery_data(&discovery_data)?;

        let config = config.into();

        let pubkey = sodoken::BufWriteSized::new_no_lock();
        let seckey = sodoken::BufWriteSized::new_mem_locked()?;
        crypto_box::keypair(pubkey.clone(), seckey.clone()).await?;

        let pubkey = Pubkey::from(pubkey);
        let seckey = Arc::new(seckey.to_read_sized());

        let state = Arc::new(Mutex::new(DiscoverState::new(pubkey, discovery_data)));

        let discover_task = tokio::task::spawn(discover_task(
            config,
            state.clone(),
            pubkey,
            seckey,
        ));

        Ok(Self {
            discover_task,
            state,
            pubkey,
        })
    }

    /// Get the local network discovery identifier for this instance.
    pub fn id(&self) -> String {
        self.pubkey.to_base64()
    }

    /// Publish new discovery data to local network peers.
    pub fn update_discovery_data(&self, discovery_data: Vec<Vec<u8>>) -> Result<()> {
        Self::check_discovery_data(&discovery_data)?;

        self.state.lock().update_discovery_data(discovery_data);

        Ok(())
    }

    // private

    fn check_discovery_data(discovery_data: &Vec<Vec<u8>>) -> Result<()> {
        if discovery_data.len() > 64 {
            return Err(Error::id("Max64DiscoveryData"));
        }

        for v in discovery_data.iter() {
            if v.len() > DISC_DATA_MAX {
                return Err(Error::str(format!("Discovery data too large {}, max: {DISC_DATA_MAX}", v.len())));
            }
        }

        Ok(())
    }
}

#[allow(dead_code)]
struct PeerState {
    socket: Weak<tokio::net::UdpSocket>,
    addr: std::net::SocketAddr,
    last_comm: std::time::Instant,
    last_query_time: Option<std::time::Instant>,
    want_state_counter: Option<u64>,
    want_state_data: Option<Vec<Option<Vec<u8>>>>,
    complete_state_counter: Option<u64>,
}

impl  PeerState {
    fn new(
        socket: Weak<tokio::net::UdpSocket>,
        addr: std::net::SocketAddr,
    ) -> Self {
        Self {
            socket,
            addr,
            last_comm: std::time::Instant::now(),
            last_query_time: None,
            want_state_counter: None,
            want_state_data: None,
            complete_state_counter: None,
        }
    }
}

struct DiscoverState {
    this_pubkey: Pubkey,
    last_announce: Option<std::time::Instant>,
    announce_int: std::time::Duration,
    last_recv: std::time::Instant,
    state_counter: u64,
    discovery_data: Vec<Vec<u8>>,
    want_abort: bool,
    peer_info: HashMap<Pubkey, PeerState>,
    send_items: Vec<SendItem>,
}

impl DiscoverState {
    fn new(this_pubkey: Pubkey, discovery_data: Vec<Vec<u8>>) -> Self {
        let mut this = Self {
            this_pubkey,
            last_announce: None,
            announce_int: std::time::Duration::from_secs(15),
            last_recv: std::time::Instant::now(),
            state_counter: 0,
            discovery_data: Vec::new(),
            want_abort: false,
            peer_info: HashMap::new(),
            send_items: Vec::new(),
        };
        this.update_discovery_data(discovery_data);
        this
    }
}

struct SendItem {
    rem_pubkey: Pubkey,
    socket: Weak<tokio::net::UdpSocket>,
    addr: std::net::SocketAddr,
    buffer: sodoken::BufRead,
}

enum DiscoverStateAction {
    Idle,
    Abort,
    Announce(Vec<u8>),
    SendItems(Vec<SendItem>),
}

impl DiscoverState {
    fn mcast_recv(
        &mut self,
        res: Result<(
            Weak<tokio::net::UdpSocket>,
            Vec<u8>,
            std::net::SocketAddr,
        )>,
    ) {
        let (socket, data, addr) = match res {
            Err(err) => {
                tracing::debug!(?err);
                self.want_abort = true;
                return;
            }
            Ok(r) => r,
        };

        self.last_recv = std::time::Instant::now();

        // NOTE: the size of the data packet was verified before
        //       this function was called, so safe to index.

        let rem_pubkey = Pubkey::from(&data[4..]);

        if rem_pubkey == self.this_pubkey {
            // we don't need to pay attention to our own announces
            return;
        }

        tracing::trace!(
            this_pubkey = ?self.this_pubkey,
            ?rem_pubkey,
            rem_addr = ?addr,
            "received announce",
        );

        self.check_peer_state(socket, addr, rem_pubkey);
    }

    fn ucast_recv(
        &mut self,
        res: Result<(
            Weak<tokio::net::UdpSocket>,
            Vec<u8>,
            std::net::SocketAddr,
        )>,
    ) {
        let (socket, data, addr) = match res {
            Err(err) => {
                tracing::debug!(?err);
                self.want_abort = true;
                return;
            }
            Ok(r) => r,
        };

        self.last_recv = std::time::Instant::now();

        // NOTE: the size of the data packet was verified before
        //       this function was called, so safe to index.

        let rem_pubkey = Pubkey::from(&data[4..36]);

        if rem_pubkey == self.this_pubkey {
            tracing::error!("Library Error: Unicast From Self!");
            // we don't need to pay attention to our own announces
            return;
        }

        tracing::trace!(
            this_pubkey = ?self.this_pubkey,
            ?rem_pubkey,
            rem_addr = ?addr,
            "received unicast",
        );

        self.check_peer_state(socket.clone(), addr, rem_pubkey);

        // TODO - DECRYPT data

        let data = &data[36..];

        // skip nonce
        let data = &data[16..];

        if data.len() < 8 {
            return;
        }

        match &data[..8] {
            b"0tx5qery" => {
                if data.len() < 24 {
                    return;
                }

                // this is actually the remote state counter
                let mut state_counter = [0; 8];
                state_counter.copy_from_slice(&data[8..16]);
                let state_counter = u64::from_le_bytes(state_counter);
                // see if we want the new state

                // ignore the query bitfield for now, send everything

                self.query_respond(socket, addr, state_counter);
            }
            _ => tracing::debug!("unexpected unicast msg type"),
        }
    }

    fn query_respond(
        &mut self,
        _socket: Weak<tokio::net::UdpSocket>,
        _addr: std::net::SocketAddr,
        _state_counter: u64,
    ) {
        todo!()
    }

    fn update_discovery_data(&mut self, discovery_data: Vec<Vec<u8>>) {
        self.discovery_data = discovery_data;

        // update our state counter, so we'll get new queries
        self.state_counter += 1;

        // trigger a new announce
        self.last_announce = None;
    }

    fn clear(&mut self) {
        self.peer_info.clear();
        self.send_items.clear();
    }

    fn cleanup(&mut self) {
        let now = std::time::Instant::now();

        self.peer_info.retain(|_, p| {
            (now - p.last_comm) < std::time::Duration::from_secs(30)
        });
    }

    fn check_peer_state(
        &mut self,
        socket: Weak<tokio::net::UdpSocket>,
        addr: std::net::SocketAddr,
        rem_pubkey: Pubkey,
    ) {
        if let Some(peer_info) = self.peer_info.get_mut(&rem_pubkey) {
            peer_info.last_comm = std::time::Instant::now();
        } else {
            if self.peer_info.len() >= 4096 {
                tracing::debug!("too many peers, ignoring");
                return;
            }
            self.peer_info.insert(rem_pubkey, PeerState::new(
                socket,
                addr,
            ));
        }
    }

    fn get_action(&mut self, pubkey: &Pubkey) -> DiscoverStateAction {
        // abort if want_abort is set (likely a recv error on the socket)
        if self.want_abort {
            self.clear();
            return DiscoverStateAction::Abort;
        }

        // if we haven't received anything in the last 30 seconds
        // we might need to re-bind our sockets. Abort for now.
        if self.last_recv.elapsed() >= std::time::Duration::from_secs(30) {
            self.clear();
            return DiscoverStateAction::Abort;
        }

        // evict old expired peers
        self.cleanup();

        // if we've passed our announce interval, announce
        if self.last_announce.map(|t| {
            t.elapsed() > self.announce_int
        }).unwrap_or(true) {
            return DiscoverStateAction::Announce(self.mark_announce(pubkey));
        }

        // check to see if we need to generate any outgoing messages
        self.process();

        // if there's any outgoing data pending, return that
        if !self.send_items.is_empty() {
            return DiscoverStateAction::SendItems(std::mem::take(&mut self.send_items));
        }

        // we had no actions to take, report idle
        DiscoverStateAction::Idle
    }

    fn process(&mut self) {
        self.process_queries();
    }

    fn process_queries(&mut self) {
        let mut send_items = Vec::new();

        for (rem_pubkey, peer_state) in self.peer_info.iter_mut() {
            // we have a complete state, and don't want a new one
            // we don't need to query
            if peer_state.complete_state_counter.is_some() && peer_state.want_state_counter.is_none() {
                continue;
            }

            // if the query hasn't completed in the last 5 seconds,
            // send another
            if peer_state.last_query_time.map(|t| {
                t.elapsed() > std::time::Duration::from_secs(5)
            }).unwrap_or(true) {
                peer_state.last_query_time = Some(std::time::Instant::now());

                let mut buf = Vec::with_capacity(8 + 8 + 8);
                buf.extend_from_slice(b"0tx5qery");
                buf.extend_from_slice(&self.state_counter.to_le_bytes());
                // just request all data fields for now
                buf.extend_from_slice(&0xffffffffffffffff_u64.to_le_bytes());

                send_items.push((*rem_pubkey, peer_state.socket.clone(), peer_state.addr, buf));

                /*
                let mut nonce = [0; 16];
                rand::thread_rng().fill(&mut nonce);

                let mut buf = Vec::with_capacity(nonce.len() + 8 + 8 + 8);
                buf.extend_from_slice(&nonce[..]);
                buf.extend_from_slice(b"0tx5qery");
                buf.extend_from_slice(&self.state_counter.to_le_bytes());
                // just request all data fields for now
                buf.extend_from_slice(&0xffffffffffffffff_u64.to_le_bytes());

                // TODO ENCRYPT buf

                let mut data = Vec::with_capacity(4 + 32 + buf.len());
                data.extend_from_slice(b"0tx5");
                data.extend_from_slice(&self.this_pubkey.0[..]);
                data.extend_from_slice(&buf[..]);

                send_items.push(SendItem {
                    socket: peer_state.socket.clone(),
                    addr: peer_state.addr,
                    data,
                });
                */
            }
        }

        for (rem_pubkey, socket, addr, buf) in send_items {
            self.pack_send(
                rem_pubkey,
                socket,
                addr,
                buf,
            );
        }
    }

    fn mark_announce(&mut self, pubkey: &Pubkey) -> Vec<u8> {
        use rand::Rng;
        self.last_announce = Some(std::time::Instant::now());
        self.announce_int = std::time::Duration::from_millis(
            rand::thread_rng().gen_range(10000..20000),
        );
        let mut out = Vec::with_capacity(4 + 32);
        out.extend_from_slice(b"0tx5");
        out.extend_from_slice(&pubkey.0[..]);
        out
    }

    fn pack_send(
        &mut self,
        rem_pubkey: Pubkey,
        socket: Weak<tokio::net::UdpSocket>,
        addr: std::net::SocketAddr,
        data: Vec<u8>,
    ) {
        use rand::Rng;
        use std::io::Write;

        let mut e = flate2::write::ZlibEncoder::new(
            Vec::new(),
            flate2::Compression::best(),
        );
        e.write_all(&data).expect("compression failure");
        let data_compressed = e.finish().expect("compression failure");

        let (is_compressed, data) = if data_compressed.len() < data.len() {
            (true, data_compressed)
        } else {
            (false, data)
        };

        if data.len() > MAX_PACKET_SIZE - HEADER_OVERHEAD - ENC_OVERHEAD {
            tracing::debug!(len = %data.len(), "tried to send too large msg");
            return;
        }

        let mut inner_header = [0; 16];
        rand::thread_rng().fill(&mut inner_header);
        if is_compressed {
            inner_header[15] = 1;
        } else {
            inner_header[15] = 0;
        }

        let buffer = sodoken::BufExtend::new_no_lock(inner_header.len() + data.len());
        {
            let mut e = buffer.extend_lock();
            e.extend_mut_from_slice(&inner_header[..]).expect("write fail");
            e.extend_mut_from_slice(&data).expect("write fail");
        }
        let buffer = buffer.to_read();

        self.send_items.push(SendItem {
            rem_pubkey,
            socket,
            addr,
            buffer,
        });
    }
}

async fn discover_task(
    config: Arc<Tx5DiscoverConfig>,
    state: Arc<Mutex<DiscoverState>>,
    pubkey: Pubkey,
    seckey: Arc<sodoken::BufReadSized<32>>,
) {
    loop {
        // if we've looped, the old peers/messages are no longer valid
        state.lock().clear();

        let state_mcast_recv = state.clone();
        let state_ucast_recv = state.clone();
        if let Ok(shotgun) = Shotgun::new(
            Arc::new(move |res| {
                // drop bad packets before we even acquire the state lock
                if let Ok((_, data, _)) = &res {
                    if data.len() != 4 + 32 || &data[0..4] != b"0tx5" {
                        // ignore malformed packet.
                        // don't even trace this,
                        // we want to be as fast as possible
                        return;
                    }
                }
                state_mcast_recv.lock().mcast_recv(res);
            }),
            Arc::new(move |res| {
                // drop bad packets before we even acquire the state lock
                if let Ok((_, data, _)) = &res {
                    // TODO add the encryption overhead to the len check
                    if data.len() < 4 + 32 || &data[0..4] != b"0tx5" {
                        // ignore malformed packet.
                        // don't even trace this,
                        // we want to be as fast as possible
                        return;
                    }
                }
                state_ucast_recv.lock().ucast_recv(res);
            }),
            config.port,
            config.multi_v4_addr,
            config.multi_v6_addr,
        )
        .await
        {
            discover_task_2(
                config.clone(),
                state.clone(),
                shotgun,
                pubkey,
                seckey.clone(),
            )
            .await;
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
    loop {
        let action = state.lock().get_action(&pubkey);
        match action {
            DiscoverStateAction::Idle => {
                // idle action, pause polling
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
            DiscoverStateAction::Abort => return,
            DiscoverStateAction::Announce(announce) => {
                if let Err(err) = shotgun.multicast(announce).await {
                    tracing::debug!(?err);
                    return;
                }
            }
            DiscoverStateAction::SendItems(send_items) => {
                for SendItem { rem_pubkey: _, socket: _, addr: _, buffer: _ } in send_items {
                    // TODO - encrypt and send the buffer
                    todo!()
                    /*
                    if let Some(socket) = socket.upgrade() {
                        if let Err(err) = socket.send_to(&data, addr).await {
                            tracing::debug!(?err);
                        }
                        // throttle sending
                        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                    }
                    */
                }
            }
        }
    }
}

trait IpAddrExt {
    fn ext_is_global(&self) -> bool;
}

impl IpAddrExt for std::net::Ipv4Addr {
    #[inline]
    fn ext_is_global(&self) -> bool {
        if u32::from_be_bytes(self.octets()) == 0xc0000009
            || u32::from_be_bytes(self.octets()) == 0xc000000a
        {
            return true;
        }
        !self.is_private()
            && !self.is_loopback()
            && !self.is_link_local()
            && !self.is_broadcast()
            && !self.is_documentation()
            // is_shared()
            && !(self.octets()[0] == 100 && (self.octets()[1] & 0b1100_0000 == 0b0100_0000))
            && !(self.octets()[0] == 192 && self.octets()[1] == 0 && self.octets()[2] == 0)
            // is_reserved()
            && !(self.octets()[0] & 240 == 240 && !self.is_broadcast())
            // is_benchmarking()
            && !(self.octets()[0] == 198 && (self.octets()[1] & 0xfe) == 18)
            && self.octets()[0] != 0
    }
}

impl IpAddrExt for std::net::Ipv6Addr {
    #[inline]
    fn ext_is_global(&self) -> bool {
        !self.is_multicast()
            && !self.is_loopback()
            //&& !self.is_unicast_link_local()
            && !((self.segments()[0] & 0xffc0) == 0xfe80)
            //&& !self.is_unique_local()
            && !((self.segments()[0] & 0xfe00) == 0xfc00)
            && !self.is_unspecified()
            //&& !self.is_documentation()
            && !((self.segments()[0] == 0x2001) && (self.segments()[1] == 0xdb8))
    }
}

impl IpAddrExt for std::net::IpAddr {
    #[inline]
    fn ext_is_global(&self) -> bool {
        match self {
            std::net::IpAddr::V4(ip) => ip.ext_is_global(),
            std::net::IpAddr::V6(ip) => ip.ext_is_global(),
        }
    }
}

#[cfg(test)]
mod test;
