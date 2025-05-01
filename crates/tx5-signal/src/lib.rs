#![deny(missing_docs)]
#![deny(unsafe_code)]
#![doc = tx5_core::__doc_header!()]
//! # tx5-signal
//!
//! Holochain webrtc signal client.
//! This is a thin wrapper around an SBD e2e crypto client.

use std::io::{Error, Result};
use std::sync::{Arc, Weak};

pub use sbd_e2e_crypto_client::{PubKey, SbdClientConfig};

mod wire;
pub use wire::*;

mod conn;
pub use conn::*;

#[cfg(test)]
mod tests;
