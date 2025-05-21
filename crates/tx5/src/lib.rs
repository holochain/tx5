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
//! - <b><i>`*`DEFAULT`*`</i></b> `backend-libdatachannel` - WebRTC library
//!   written in C++.
//!   - [https://github.com/paullouisageneau/libdatachannel](https://github.com/paullouisageneau/libdatachannel)
//! - `backend-go-pion` - The pion webrtc library
//!   written in Go (golang).
//!   - [https://github.com/pion/webrtc](https://github.com/pion/webrtc)
//!
//! The go pion library was the original implementation, but as libdatachannel
//! has reached stability, we have switched it over to be the default as
//! it is much easier to write rust FFI bindings to C++ code than Go code.

pub use tx5_connection::Tx5InitConfig;

pub use tx5_connection::{IceServers, WebRtcConfig};

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

pub mod backend;
use backend::*;

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

// #[cfg(test)]
// mod test_behavior;
