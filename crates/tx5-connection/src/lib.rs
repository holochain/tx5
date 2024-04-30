#![deny(missing_docs)]
#![deny(unsafe_code)]
#![doc = tx5_core::__doc_header!()]
//! # tx5-connection
//!
//! Holochain webrtc connection.
//! Starts by sending messages over the sbd signal server, if we can
//! upgrade to a proper webrtc p2p connection, we do so.

use std::collections::HashMap;
use std::io::{Error, Result};
use std::sync::{Arc, Weak};

pub use tx5_signal::PubKey;

mod hub;
pub use hub::*;

mod conn;
pub use conn::*;

mod proto;

mod framed;
pub use framed::*;

#[cfg(test)]
mod test;
