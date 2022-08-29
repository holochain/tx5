#![deny(missing_docs)]
#![deny(warnings)]
#![deny(unsafe_code)]

//! Holochain webrtc signal server.
//!
//! [![Project](https://img.shields.io/badge/project-holochain-blue.svg?style=flat-square)](http://holochain.org/)
//! [![Forum](https://img.shields.io/badge/chat-forum%2eholochain%2enet-blue.svg?style=flat-square)](https://forum.holochain.org)
//! [![Chat](https://img.shields.io/badge/chat-chat%2eholochain%2enet-blue.svg?style=flat-square)](https://chat.holochain.org)
//!
//! [![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)
//! [![License: Apache-2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://www.apache.org/licenses/LICENSE-2.0)

#![doc = include_str!("docs/srv_help.md")]

/// Re-exported dependencies.
pub mod deps {
    pub use tx4_core::deps::*;
}

pub use tx4_core::{Error, ErrorExt, Id, Result};

use clap::Parser;

mod config;
pub use config::*;

mod server;
pub use server::*;
