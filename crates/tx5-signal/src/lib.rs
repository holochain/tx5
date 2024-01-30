#![deny(missing_docs)]
#![deny(unsafe_code)]
#![doc = tx5_core::__doc_header!()]
//! # tx5-signal
//!
//! Holochain webrtc signal server / client.

/// Re-exported dependencies.
pub mod deps {
    pub use hc_seed_bundle::dependencies::sodoken;
    pub use lair_keystore_api;
    pub use lair_keystore_api::dependencies::hc_seed_bundle;
    pub use tx5_core::deps::*;
}

use deps::*;

pub use tx5_core::{Error, ErrorExt, Id, Result};

use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

#[allow(dead_code)]
pub(crate) static WS_CONFIG: WebSocketConfig = WebSocketConfig {
    max_send_queue: Some(tx5_core::ws::MAX_SEND_QUEUE),
    max_message_size: Some(tx5_core::ws::MAX_MESSAGE_SIZE),
    max_frame_size: Some(tx5_core::ws::MAX_FRAME_SIZE),
    accept_unmasked_frames: false,
};

pub(crate) fn tcp_configure(
    socket: tokio::net::TcpStream,
) -> Result<tokio::net::TcpStream> {
    let socket = socket.into_std()?;
    let socket = socket2::Socket::from(socket);

    let keepalive = socket2::TcpKeepalive::new()
        .with_time(std::time::Duration::from_secs(7))
        .with_interval(std::time::Duration::from_secs(7));

    // we'll close unresponsive connections after 21-28 seconds (7 * 3)
    // (it's a little unclear how long it'll wait after the final probe)
    #[cfg(any(target_os = "linux", target_vendor = "apple"))]
    let keepalive = keepalive.with_retries(3);

    socket.set_tcp_keepalive(&keepalive)?;

    let socket = std::net::TcpStream::from(socket);
    tokio::net::TcpStream::from_std(socket)
}

mod cli;
pub use cli::*;

#[cfg(test)]
mod tests;
