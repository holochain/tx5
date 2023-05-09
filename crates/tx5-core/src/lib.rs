#![deny(missing_docs)]
#![deny(unsafe_code)]
#![deny(warnings)]
#![doc = include_str!("README.tpl")]
//! # tx5-core
//!
//! Holochain WebRTC p2p communication ecosystem core types.

include!(concat!(env!("OUT_DIR"), "/readme.rs"));

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

mod uniq;
pub use uniq::*;

mod url;
pub use crate::url::*;

#[cfg(feature = "file_check")]
pub mod file_check;

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

/// Pinned, boxed, future type alias.
pub type BoxFut<'lt, T> =
    std::pin::Pin<Box<dyn std::future::Future<Output = T> + 'lt + Send>>;
