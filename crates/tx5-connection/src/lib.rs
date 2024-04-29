#![deny(missing_docs)]
#![deny(unsafe_code)]
#![doc = tx5_core::__doc_header!()]
//! # tx5-connection
//!
//! Holochain webrtc connection.
//! Starts by sending messages over the sbd signal server, if we can
//! upgrade to a proper webrtc p2p connection, we do so.

use std::io::Result;
use std::sync::Arc;

pub use tx5_signal::PubKey;

/// A signal server connection from which we can establish tx5 connections.
pub struct Tx5ConnectionHub {
}

impl Tx5ConnectionHub {
    /// Create a new Tx5ConnectionHub based off a connected tx5 signal client.
    /// Note, if this is not a "listener" client,
    /// you do not need to ever call accept.
    pub async fn new(_client: tx5_signal::SignalConnection) -> Result<Self> {
        todo!()
    }

    /// Establish a connection to a remote peer.
    /// Note, if there is already an open connection, this Arc will point
    /// to that same connection instance.
    pub async fn connect(&self, _peer_pub_key: PubKey) -> Result<Arc<Tx5Connection>> {
        todo!()
    }

    /// Accept an incoming tx5 connection.
    pub async fn accept(&self) -> Option<Arc<Tx5Connection>> {
        todo!()
    }
}

/// A tx5 connection.
pub struct Tx5Connection {
}

impl Tx5Connection {
    /// The pub key of the remote peer this is connected to.
    pub fn pub_key(&self) -> &PubKey {
        todo!()
    }

    /// Send up to 16KiB of message data.
    pub async fn send(&self, _msg: &[u8]) -> Result<()> {
        todo!()
    }

    /// Receive up to 16KiB of message data.
    pub async fn recv(&self) -> Option<Vec<u8>> {
        todo!()
    }
}
