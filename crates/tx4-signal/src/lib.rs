#![deny(missing_docs)]
#![deny(warnings)]
#![deny(unsafe_code)]

//! Holochain webrtc signal server / client.
//!
//! [![Project](https://img.shields.io/badge/project-holochain-blue.svg?style=flat-square)](http://holochain.org/)
//! [![Forum](https://img.shields.io/badge/chat-forum%2eholochain%2enet-blue.svg?style=flat-square)](https://forum.holochain.org)
//! [![Chat](https://img.shields.io/badge/chat-chat%2eholochain%2enet-blue.svg?style=flat-square)](https://chat.holochain.org)
//!
//! [![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)
//! [![License: Apache-2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://www.apache.org/licenses/LICENSE-2.0)
//!

/// Re-exported dependencies.
pub mod deps {
    pub use hc_seed_bundle::dependencies::sodoken;
    pub use lair_keystore_api;
    pub use lair_keystore_api::dependencies::hc_seed_bundle;
    pub use tx4_core::deps::*;
}

use deps::*;

pub use tx4_core::{Error, ErrorExt, Id, Result};

use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

#[allow(dead_code)]
pub(crate) static WS_CONFIG: WebSocketConfig = WebSocketConfig {
    max_send_queue: Some(tx4_core::ws::MAX_SEND_QUEUE),
    max_message_size: Some(tx4_core::ws::MAX_MESSAGE_SIZE),
    max_frame_size: Some(tx4_core::ws::MAX_FRAME_SIZE),
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

pub mod tls;

#[cfg(test)]
mod tests;
