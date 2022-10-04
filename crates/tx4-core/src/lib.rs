#![deny(missing_docs)]
#![deny(unsafe_code)]
#![deny(warnings)]

//! Holochain WebRTC P2P Communication Ecosystem Core Types.
//!
//! [![Project](https://img.shields.io/badge/project-holochain-blue.svg?style=flat-square)](http://holochain.org/)
//! [![Forum](https://img.shields.io/badge/chat-forum%2eholochain%2enet-blue.svg?style=flat-square)](https://forum.holochain.org)
//! [![Chat](https://img.shields.io/badge/chat-chat%2eholochain%2enet-blue.svg?style=flat-square)](https://chat.holochain.org)
//!
//! [![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)
//! [![License: Apache-2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://www.apache.org/licenses/LICENSE-2.0)

/// Re-exported dependencies.
pub mod deps {
    pub use base64;
    pub use serde;
    pub use serde_json;
}

mod error;
pub use error::*;

mod id;
pub use id::*;

mod url;
pub use crate::url::*;

pub mod wire;

/// Websocket configuration constants.
pub mod ws {
    /// Outgoing message queue size.
    pub const MAX_SEND_QUEUE: usize = 32;

    /// Max incoming and outgoing message size.
    pub const MAX_MESSAGE_SIZE: usize = 2048;

    /// Max incoming and outgoing frame size.
    pub const MAX_FRAME_SIZE: usize = 2048;
}
