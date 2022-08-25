//! Tx4-signal wire-level protocol encoding utilities.

use crate::*;
use lair_keystore_api::prelude::*;

/// Tx4 signal server wire protocol for client to server communication.
#[derive(Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SrvWire {
    /// When a client connects to a server, the server sends an initial
    /// authentication / hello request.
    #[serde(rename_all = "camelCase")]
    AuthReqV1 {
        /// The x25519 public key of this server.
        srv_pub: X25519PubKey,

        /// The crypto box nonce for the encoded con_key.
        nonce: BinDataSized<24>,

        /// An encoded con_key that must be returned in the AuthResV1 response.
        cipher: BinData,

        /// iceServers configuration associated with this webrtc signal server.
        ice: serde_json::Value,
    },

    /// On receiving an AuthReqV1, a client should respond with this AuthResV1.
    #[serde(rename_all = "camelCase")]
    AuthResV1 {
        /// The decoded con_key proving we have a private key associated with
        /// our public key.
        con_key: BinDataSized<32>,

        /// If we would like the server to consider us addressable, send true
        /// here. A server may have a limited number of addressable slots.
        req_addr: bool,
    },

    /// Once the auth handshake is complete, clients may begin sending forward
    /// messages to other clients, and the server may begin sending forward
    /// messages from other clients.
    #[serde(rename_all = "camelCase")]
    FwdV1 {
        /// On client-to-server, this is the destination pub key.
        /// On server-to-client, this is the source pub key.
        rem_pub: BinDataSized<32>,

        /// The data to be forwarded / that has been forwarded.
        #[serde(with = "serde_bytes")]
        data: Vec<u8>,
    },

    /// As a standin for bootstrapping, clients may trigger "demo" broadcasts
    /// announcing themselves to everyone else connected on the signal server.
    /// If "demo" mode is not enabled, sending a demo command may cause a ban.
    #[serde(rename_all = "camelCase")]
    DemoV1 {
        /// The client pub key to announce.
        rem_pub: BinDataSized<32>,
    },
}

impl SrvWire {
    /// Decode from wire binary format (msgpack).
    pub fn decode(wire: &[u8]) -> Result<Self> {
        rmp_serde::from_slice(wire).map_err(Error::err)
    }

    /// Encode into wire binary format (msgpack).
    pub fn encode(&self) -> Result<Vec<u8>> {
        rmp_serde::to_vec_named(&self).map_err(Error::err)
    }
}

/// Tx4 signal client wire protocol for client to client communication.
#[derive(Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CliWire {
    /// WebRTC offer.
    #[serde(rename_all = "camelCase")]
    OfferV1(serde_json::Value),

    /// WebRTC answer.
    #[serde(rename_all = "camelCase")]
    AnswerV1(serde_json::Value),

    /// WebRTC ICE candidate.
    #[serde(rename_all = "camelCase")]
    IceV1(serde_json::Value),

    /// Demo mode bootstrapping standin.
    /// If demo messages are not authorized on a server, sending a demo
    /// message could result in a ban.
    #[serde(rename_all = "camelCase")]
    DemoV1(BinDataSized<32>),
}

impl CliWire {
    /// Decode from wire binary format (msgpack).
    pub fn decode(wire: &[u8]) -> Result<Self> {
        rmp_serde::from_slice(wire).map_err(Error::err)
    }

    /// Encode into wire binary format (msgpack).
    pub fn encode(&self) -> Result<Vec<u8>> {
        rmp_serde::to_vec_named(&self).map_err(Error::err)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn test_wire_encode_decode() {
        let src = SrvWire::FwdV1 {
            rem_pub: [0; 32].into(),
            data: vec![1; 2].into_boxed_slice().into(),
        };
        let dst = SrvWire::decode(&src.encode().unwrap()).unwrap();
        assert_eq!(src, dst);
    }
}
