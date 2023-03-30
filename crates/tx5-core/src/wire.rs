//! Tx5-signal wire-level protocol encoding utilities.

use crate::*;

/// 24 byte nonce for crypto_box cipher.
#[derive(Clone, Debug, Copy)]
pub struct Nonce(pub [u8; 24]);

impl serde::Serialize for Nonce {
    fn serialize<S>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let s = base64::encode_config(self.0, base64::URL_SAFE_NO_PAD);
        serializer.serialize_str(&s)
    }
}

impl<'de> serde::Deserialize<'de> for Nonce {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let v = base64::decode_config(s, base64::URL_SAFE_NO_PAD)
            .map_err(serde::de::Error::custom)?;
        if v.len() != 24 {
            return Err(serde::de::Error::custom(Error::id("InvalidLen")));
        }
        let mut out = [0; 24];
        out.copy_from_slice(&v);
        Ok(Self(out))
    }
}

impl From<[u8; 24]> for Nonce {
    fn from(f: [u8; 24]) -> Self {
        Self(f)
    }
}

/// Cipher data for crypto_box.
#[derive(Clone, Debug)]
pub struct Cipher(pub Box<[u8]>);

impl serde::Serialize for Cipher {
    fn serialize<S>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let s = base64::encode_config(&self.0, base64::URL_SAFE_NO_PAD);
        serializer.serialize_str(&s)
    }
}

impl<'de> serde::Deserialize<'de> for Cipher {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(Self(
            base64::decode_config(s, base64::URL_SAFE_NO_PAD)
                .map_err(serde::de::Error::custom)?
                .into_boxed_slice(),
        ))
    }
}

impl From<Box<[u8]>> for Cipher {
    fn from(f: Box<[u8]>) -> Self {
        Self(f)
    }
}

/// Tx5 signal server wire protocol for client to server communication.
#[derive(Debug, serde::Deserialize, serde::Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Wire {
    /// When a client connects to a server, the server sends an initial
    /// authentication / hello request.
    #[serde(rename_all = "camelCase")]
    AuthReqV1 {
        /// The x25519 public key of this server.
        srv_pub: Id,

        /// The crypto box nonce for the encoded con_key.
        nonce: Nonce,

        /// An encoded con_key that must be returned in the AuthResV1 response.
        cipher: Cipher,

        /// iceServers configuration associated with this webrtc signal server.
        ice: serde_json::Value,
    },

    /// On receiving an AuthReqV1, a client should respond with this AuthResV1.
    #[serde(rename_all = "camelCase")]
    AuthResV1 {
        /// The decoded con_key proving we have a private key associated with
        /// our public key.
        con_key: Id,

        /// If we would like the server to consider us addressable, send true
        /// here. A server may have a limited number of addressable slots.
        req_addr: bool,
    },

    /// A client sends an encrypted message to another client.
    #[serde(rename_all = "camelCase")]
    FwdV1 {
        /// The remote id to send this message to,
        /// or that this message came from.
        rem_pub: Id,

        /// The crypto box nonce for the encoded FwdInnerV1 message.
        nonce: Nonce,

        /// The cipher bytes for the encoded FwdInnerV1 message.
        cipher: Cipher,
    },

    /// As a standin for bootstrapping, clients may trigger "demo" broadcasts
    /// announcing themselves to everyone else connected on the signal server.
    /// If "demo" mode is not enabled, sending a demo command may cause a ban.
    #[serde(rename_all = "camelCase")]
    DemoV1 {
        /// The client pub key to announce.
        rem_pub: Id,
    },
}

impl Wire {
    /// Decode from wire format (json).
    pub fn decode(wire: &[u8]) -> Result<Self> {
        serde_json::from_slice(wire).map_err(Error::err)
    }

    /// Encode into wire format (json).
    pub fn encode(&self) -> Result<Vec<u8>> {
        serde_json::to_string(&self)
            .map_err(Error::err)
            .map(|s| s.into_bytes())
    }
}

/// The internal message that is encrypted and forwarded between clients.
#[derive(Debug, serde::Deserialize, serde::Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum FwdInnerV1 {
    /// WebRTC offer.
    #[serde(rename_all = "camelCase")]
    Offer {
        /// Sequence.
        seq: f64,

        /// WebRTC offer.
        offer: serde_json::Value,
    },

    /// WebRTC answer.
    #[serde(rename_all = "camelCase")]
    Answer {
        /// Sequence.
        seq: f64,

        /// WebRTC answer.
        answer: serde_json::Value,
    },

    /// WebRTC ICE candidate.
    #[serde(rename_all = "camelCase")]
    Ice {
        /// Sequence.
        seq: f64,

        /// WebRTC ICE candidate.
        ice: serde_json::Value,
    },
}

impl FwdInnerV1 {
    /// Decode from wire format (json).
    pub fn decode(wire: &[u8]) -> Result<Self> {
        serde_json::from_slice(wire).map_err(Error::err)
    }

    /// Encode into wire format (json).
    pub fn encode(&self) -> Result<Vec<u8>> {
        serde_json::to_string(&self)
            .map_err(Error::err)
            .map(|s| s.into_bytes())
    }

    /// Set the seq.
    pub fn set_seq(&mut self, new_seq: f64) {
        match self {
            FwdInnerV1::Offer { seq, .. } => *seq = new_seq,
            FwdInnerV1::Answer { seq, .. } => *seq = new_seq,
            FwdInnerV1::Ice { seq, .. } => *seq = new_seq,
        }
    }

    /// Get the seq.
    pub fn get_seq(&self) -> f64 {
        match self {
            FwdInnerV1::Offer { seq, .. } => *seq,
            FwdInnerV1::Answer { seq, .. } => *seq,
            FwdInnerV1::Ice { seq, .. } => *seq,
        }
    }
}
