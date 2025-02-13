use crate::*;

const F_HREQ: &[u8] = b"hreq";
const F_HRES: &[u8] = b"hres";
const F_OFFR: &[u8] = b"offr";
const F_ANSW: &[u8] = b"answ";
const F_ICEM: &[u8] = b"icem";
const F_FMSG: &[u8] = b"fmsg";
const F_KEEP: &[u8] = b"keep";

/// Parsed signal message.
pub enum SignalMessage {
    /// Initiate a handshake with a peer.
    HandshakeReq([u8; 32]),

    /// Complete a handshake with a peer.
    HandshakeRes([u8; 32]),

    /// As an impolite node, send a webrtc offer.
    Offer(Vec<u8>),

    /// As a polite node, send a webrtc answer.
    Answer(Vec<u8>),

    /// Webrtc connectivity message.
    Ice(Vec<u8>),

    /// Pre-webrtc and webrtc failure fallback communication message.
    Message(Vec<u8>),

    /// Keepalive
    Keepalive,

    /// Message type not understood by this client.
    Unknown,
}

impl std::fmt::Debug for SignalMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::HandshakeReq(_) => f.write_str("HandshakeReq"),
            Self::HandshakeRes(_) => f.write_str("HandshakeRes"),
            Self::Offer(_) => f.write_str("Offer"),
            Self::Answer(_) => f.write_str("Answer"),
            Self::Ice(_) => f.write_str("Ice"),
            Self::Message(_) => f.write_str("Message"),
            Self::Keepalive => f.write_str("Keepalive"),
            Self::Unknown => f.write_str("Unknown"),
        }
    }
}

impl SignalMessage {
    /// Initiate a handshake with a peer.
    pub(crate) fn handshake_req() -> ([u8; 32], Vec<u8>) {
        use rand::Rng;

        let mut nonce = [0; 32];
        rand::thread_rng().fill(&mut nonce[..]);

        let mut out = Vec::with_capacity(4 + 32);
        out.extend_from_slice(F_HREQ);
        out.extend_from_slice(&nonce);

        (nonce, out)
    }

    /// Complete a handshake with a peer.
    pub(crate) fn handshake_res(nonce: [u8; 32]) -> Vec<u8> {
        let mut out = Vec::with_capacity(4 + 32);
        out.extend_from_slice(F_HRES);
        out.extend_from_slice(&nonce);

        out
    }

    /// As an impolite node, send a webrtc offer.
    pub(crate) fn offer(mut offer: Vec<u8>) -> Result<Vec<u8>> {
        offer.splice(0..0, F_OFFR.iter().cloned());
        Ok(offer)
    }

    /// As a polite node, send a webrtc answer.
    pub(crate) fn answer(mut answer: Vec<u8>) -> Result<Vec<u8>> {
        answer.splice(0..0, F_ANSW.iter().cloned());
        Ok(answer)
    }

    /// Webrtc connectivity message.
    pub(crate) fn ice(mut ice: Vec<u8>) -> Result<Vec<u8>> {
        ice.splice(0..0, F_ICEM.iter().cloned());
        Ok(ice)
    }

    /// Pre-webrtc and webrtc failure fallback communication message.
    pub(crate) fn message(mut msg: Vec<u8>) -> Result<Vec<u8>> {
        if msg.len() > 16 * 1024 {
            return Err(Error::other("msg too long"));
        }
        msg.splice(0..0, F_FMSG.iter().cloned());
        Ok(msg)
    }

    /// Keepalive.
    pub(crate) fn keepalive() -> Vec<u8> {
        F_KEEP.to_vec()
    }

    /// Parse a raw received buffer into a signal message.
    pub(crate) fn parse(mut b: Vec<u8>) -> Result<Self> {
        if b.len() < 4 {
            return Err(Error::other("msg too short"));
        }
        match &b[..4] {
            F_HREQ => {
                if b.len() != 4 + 32 {
                    return Err(Error::other("invalid hreq"));
                }
                let mut nonce = [0; 32];
                nonce.copy_from_slice(&b[4..]);
                Ok(SignalMessage::HandshakeReq(nonce))
            }
            F_HRES => {
                if b.len() != 4 + 32 {
                    return Err(Error::other("invalid hres"));
                }
                let mut nonce = [0; 32];
                nonce.copy_from_slice(&b[4..]);
                Ok(SignalMessage::HandshakeRes(nonce))
            }
            F_OFFR => {
                let _ = b.drain(..4);
                Ok(SignalMessage::Offer(b))
            }
            F_ANSW => {
                let _ = b.drain(..4);
                Ok(SignalMessage::Answer(b))
            }
            F_ICEM => {
                let _ = b.drain(..4);
                Ok(SignalMessage::Ice(b))
            }
            F_FMSG => {
                let _ = b.drain(..4);
                Ok(SignalMessage::Message(b))
            }
            F_KEEP => Ok(SignalMessage::Keepalive),
            _ => Ok(SignalMessage::Unknown),
        }
    }
}
