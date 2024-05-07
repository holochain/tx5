#![deny(missing_docs)]
#![deny(unsafe_code)]
#![doc = tx5_core::__doc_header!()]
//! # tx5
//!
//! Tx5 - The main holochain tx5 webrtc networking crate.
//!
//! # WebRTC Backend Features
//!
//! Tx5 can be backed currently by 1 of 2 backend webrtc libraries.
//!
//! - <b><i>`*`DEFAULT`*`</i></b> `backend-go-pion` - The pion webrtc library
//!   writen in go (golang).
//!   - [https://github.com/pion/webrtc](https://github.com/pion/webrtc)
//! - `backend-webrtc-rs` - The rust webrtc library.
//!   - [https://github.com/webrtc-rs/webrtc](https://github.com/webrtc-rs/webrtc)
//!
//! The go pion library is currently the default as it is more mature
//! and well tested, but comes with some overhead of calling into a different
//! memory/runtime. When the rust library is stable enough for holochain's
//! needs, we will switch the default. To switch now, or if you want to
//! make sure the backend doesn't change out from under you, set
//! no-default-features and explicitly enable the backend of your choice.

use std::collections::HashMap;
use std::io::{Error, Result};
use std::sync::{Arc, Mutex, Weak};

pub use tx5_connection::tx5_signal::PubKey;

mod url;
pub use url::*;

/// Dynamic future type.
pub type BoxFuture<'lt, T> =
    std::pin::Pin<Box<dyn std::future::Future<Output = T> + 'lt + Send>>;

/// Callback in charge of sending preflight data if any.
pub type PreflightSendCb = Arc<
    dyn Fn(&PeerUrl) -> BoxFuture<'static, Result<Vec<u8>>>
        + 'static
        + Send
        + Sync,
>;

/// Callback in charge of validating preflight data if any.
pub type PreflightCheckCb = Arc<
    dyn Fn(&PeerUrl, Vec<u8>) -> BoxFuture<'static, Result<()>>
        + 'static
        + Send
        + Sync,
>;

mod config;
pub use config::*;

mod sig;
pub(crate) use sig::*;

mod peer;
pub(crate) use peer::*;

mod ep;
pub use ep::*;

pub mod stats;

#[cfg(test)]
mod test;

//pub use tx5_core::Tx5InitConfig;

// #[cfg(test)]
// mod test_behavior;
