//! Types associated with the Tx5 protocol.

use crate::BackBuf;
use tx5_core::{Error, Result};

/// Tx5 protocol message payload max size.
pub const MAX_PAYLOAD: u32 = 0b00111111111111111111111111111111;

/// 4 byte Tx5 protocol header.
#[derive(Debug, PartialEq)]
pub enum ProtoHeader {
    /// This is a protocol version handshake.
    /// `2 bit zero / the protocol version`.
    Version(u32),

    /// The remainder of this chunk represents the entirety of a message.
    /// `2 bit one / the size of this chunk`.
    CompleteMessage(u32),

    /// This is a request for a permit to send a message of size larger than
    /// a single chunk.
    /// `2 bit two / the size of the message`.
    PermitRequest(u32),

    /// This is an authorization to proceed with sending payload chunks.
    /// `2 bit three / the size of the message`.
    PermitGrant(u32),
}

impl ProtoHeader {
    /// Decode 4 bytes into a Tx5 protocol header.
    pub fn decode(a: u8, b: u8, c: u8, d: u8) -> Self {
        let r = u32::from_be_bytes(*&[a, b, c, d]);
        use bit_field::BitField;
        match r.get_bits(30..) {
            0 => Self::Version(r.get_bits(..30)),
            1 => Self::CompleteMessage(r.get_bits(..30)),
            2 => Self::PermitRequest(r.get_bits(..30)),
            3 => Self::PermitGrant(r.get_bits(..30)),
            _ => unreachable!(),
        }
    }

    /// Encode a Tx5 protocol header into canonical 4 bytes.
    pub fn encode(&self) -> Result<(u8, u8, u8, u8)> {
        use bit_field::BitField;
        let mut out: u32 = 0;
        match self {
            Self::Version(v) => {
                if *v > MAX_PAYLOAD {
                    return Err(Error::id("VersionOverflow"));
                }
                out.set_bits(30.., 0);
                out.set_bits(..30, *v);
            }
            Self::CompleteMessage(s) => {
                if *s > MAX_PAYLOAD {
                    return Err(Error::id("SizeOverflow"));
                }
                out.set_bits(30.., 1);
                out.set_bits(..30, *s);
            }
            Self::PermitRequest(s) => {
                if *s > MAX_PAYLOAD {
                    return Err(Error::id("SizeOverflow"));
                }
                out.set_bits(30.., 2);
                out.set_bits(..30, *s);
            }
            Self::PermitGrant(s) => {
                if *s > MAX_PAYLOAD {
                    return Err(Error::id("SizeOverflow"));
                }
                out.set_bits(30.., 3);
                out.set_bits(..30, *s);
            }
        }
        let out = out.to_be_bytes();
        Ok((out[0], out[1], out[2], out[3]))
    }
}

/// Result of encoding a message into the Tx5 protocol.
pub enum ProtoEncodeResult {
    /// We need to request a permit. Send the permit request first,
    /// once we receive the authorization, forward the rest of
    /// the message payload.
    NeedPermit {
        /// First, request a permit to send the payload.
        permit_req: BackBuf,

        /// Second, send the actual payload chunks.
        msg_payload: Vec<BackBuf>,
    },

    /// This message fit in a single payload chunk. We do not need
    /// to request a permit ahead of time, so just send the chunk.
    OneMessage(BackBuf),
}

/// Encode some data into the Tx5 protocol.
pub fn proto_encode(data: &[u8]) -> Result<ProtoEncodeResult> {
    const MAX: usize = (16 * 1024) - 4;
    let len = data.len();

    if len > MAX_PAYLOAD as usize {
        return Err(Error::id("PayloadSizeOverflow"));
    }

    if len <= MAX {
        let (a, b, c, d) = ProtoHeader::CompleteMessage(len as u32).encode()?;

        let mut go_buf = tx5_go_pion::GoBuf::new()?;
        go_buf.reserve(len + 4)?;
        go_buf.extend(&[a, b, c, d])?;
        go_buf.extend(data)?;

        Ok(ProtoEncodeResult::OneMessage(BackBuf::from_raw(go_buf)))
    } else {
        let (a, b, c, d) = ProtoHeader::PermitRequest(len as u32).encode()?;
        let permit_req = BackBuf::from_slice(&[a, b, c, d])?;

        let mut msg_payload = Vec::new();
        let mut cur = 0;

        while len - cur > 0 {
            let amt = std::cmp::min(16 * 1024, len - cur);
            msg_payload.push(BackBuf::from_slice(&data[cur..cur + amt])?);
            cur += amt;
        }

        Ok(ProtoEncodeResult::NeedPermit {
            permit_req,
            msg_payload,
        })
    }
}

/// Result of decoding an incoming message chunk.
#[derive(Debug, PartialEq)]
pub enum ProtoDecodeResult {
    /// Nothing needs to happen at the moment... continue receiving chunks.
    Idle,

    /// Received incoming message.
    Message(Vec<u8>),

    /// The remote node is requesting a permit to send us chunks of data.
    PermitRequest(u32),

    /// The remote node has granted us a permit to send them chunks of data.
    PermitGrant(u32),
}

#[derive(Clone, Copy)]
enum DecodeState {
    NeedVersion,
    Ready,
    /// The REMOTE requested a permit from US.
    /// Totally different from when we make a permit request of the remote : )
    AwaitThisNodePermit(u32),
    ReceiveChunked,
}

/// Tx5 protocol decoder.
pub struct ProtoDecoder {
    state: DecodeState,
    want_size: usize,
    incoming: Vec<u8>,
}

impl Default for ProtoDecoder {
    fn default() -> Self {
        Self {
            state: DecodeState::NeedVersion,
            want_size: 0,
            incoming: Vec::new(),
        }
    }
}

impl ProtoDecoder {
    /// Notify the decoder that we sent the previously requested permit
    /// to the remote.
    pub fn sent_permit(&mut self) -> Result<()> {
        if let DecodeState::AwaitThisNodePermit(permit_len) = self.state {
            self.state = DecodeState::ReceiveChunked;
            self.want_size = permit_len as usize;
            self.incoming.reserve(self.want_size);
            Ok(())
        } else {
            Err(Error::id("InvalidStateToSendPermit"))
        }
    }

    /// Process the next incoming chunk from the remote.
    pub fn decode(&mut self, mut chunk: BackBuf) -> Result<ProtoDecodeResult> {
        match self.state {
            DecodeState::NeedVersion => {
                const WANT_VERSION: ProtoHeader = ProtoHeader::Version(1);

                if chunk.len()? != 4 {
                    return Err(Error::id("InvalidVersionHeaderLen"));
                }

                let hdr = chunk.to_vec()?;
                let hdr = ProtoHeader::decode(hdr[0], hdr[1], hdr[2], hdr[3]);
                if hdr != WANT_VERSION {
                    return Err(Error::err(format!(
                        "Invalid remote Tx5 protocol version: {hdr:?}"
                    )));
                }

                self.state = DecodeState::Ready;

                Ok(ProtoDecodeResult::Idle)
            }
            DecodeState::Ready => chunk.imp.buf.access(|buf| {
                let buf = buf?;
                let len = buf.len();

                if len < 4 {
                    return Err(Error::id("InvalidHeaderLen"));
                }

                match ProtoHeader::decode(buf[0], buf[1], buf[2], buf[3]) {
                    ProtoHeader::CompleteMessage(msg_len) => {
                        if msg_len as usize != len - 4 {
                            return Err(Error::id("InvalidCompleteMessageLen"));
                        }

                        Ok(ProtoDecodeResult::Message(buf[4..].to_vec()))
                    }
                    ProtoHeader::PermitRequest(permit_len) => {
                        self.state =
                            DecodeState::AwaitThisNodePermit(permit_len);
                        Ok(ProtoDecodeResult::PermitRequest(permit_len))
                    }
                    ProtoHeader::PermitGrant(permit_len) => {
                        Ok(ProtoDecodeResult::PermitGrant(permit_len))
                    }
                    _ => Err(Error::id("InvalidTx5ProtocolHeader")),
                }
            }),
            DecodeState::AwaitThisNodePermit(_) => {
                chunk.imp.buf.access(|buf| {
                    let buf = buf?;
                    let len = buf.len();

                    if len < 4 {
                        return Err(Error::id("InvalidHeaderLen"));
                    }
                    match ProtoHeader::decode(buf[0], buf[1], buf[2], buf[3]) {
                        ProtoHeader::PermitGrant(permit_len) => {
                            Ok(ProtoDecodeResult::PermitGrant(permit_len))
                        }
                        _ => Err(Error::id(
                            "InvalidTx5ProtocolHeaderWhileAwaitingPermit",
                        )),
                    }
                })
            }
            DecodeState::ReceiveChunked => chunk.imp.buf.access(|buf| {
                let buf = buf?;
                let len = buf.len();

                if len + self.incoming.len() > self.want_size {
                    return Err(Error::id("ChunkTooLarge"));
                }

                self.incoming.extend_from_slice(buf);

                if self.incoming.len() == self.want_size {
                    self.state = DecodeState::Ready;
                    Ok(ProtoDecodeResult::Message(std::mem::take(
                        &mut self.incoming,
                    )))
                } else {
                    Ok(ProtoDecodeResult::Idle)
                }
            }),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn proto_header_encode_decode() {
        fn check(hdr: ProtoHeader) {
            let (a, b, c, d) = hdr.encode().unwrap();
            let res = ProtoHeader::decode(a, b, c, d);
            assert_eq!(hdr, res);
        }

        for v in &[0, 42, 0b00111111111111111111111111111111] {
            check(ProtoHeader::Version(*v));
            check(ProtoHeader::CompleteMessage(*v));
            check(ProtoHeader::PermitRequest(*v));
            check(ProtoHeader::PermitGrant(*v));
        }
    }

    #[test]
    fn proto_header_overflow() {
        assert!(ProtoHeader::Version(u32::MAX).encode().is_err());
        assert!(ProtoHeader::CompleteMessage(u32::MAX).encode().is_err());
        assert!(ProtoHeader::PermitRequest(u32::MAX).encode().is_err());
        assert!(ProtoHeader::PermitGrant(u32::MAX).encode().is_err());
    }

    #[test]
    fn proto_header_version_1() {
        const PROTO_VERSION_1: &[u8; 4] =
            &[0b00000000, 0b00000000, 0b00000000, 0b00000001];

        let (a, b, c, d) = ProtoHeader::Version(1).encode().unwrap();
        assert_eq!(PROTO_VERSION_1[0], a);
        assert_eq!(PROTO_VERSION_1[1], b);
        assert_eq!(PROTO_VERSION_1[2], c);
        assert_eq!(PROTO_VERSION_1[3], d);
    }

    #[test]
    fn proto_decode_complete_msg() {
        let mut dec = ProtoDecoder::default();
        let (a, b, c, d) = ProtoHeader::Version(1).encode().unwrap();
        assert_eq!(
            ProtoDecodeResult::Idle,
            dec.decode(BackBuf::from_slice(&[a, b, c, d]).unwrap())
                .unwrap(),
        );
        match proto_encode(b"hello").unwrap() {
            ProtoEncodeResult::OneMessage(buf) => {
                match dec.decode(buf).unwrap() {
                    ProtoDecodeResult::Message(msg) => {
                        assert_eq!(b"hello", msg.as_slice());
                    }
                    _ => panic!(),
                }
            }
            _ => panic!(),
        }
    }

    #[test]
    fn proto_decode_chunked_msg() {
        let mut dec = ProtoDecoder::default();
        let (a, b, c, d) = ProtoHeader::Version(1).encode().unwrap();
        assert_eq!(
            ProtoDecodeResult::Idle,
            dec.decode(BackBuf::from_slice(&[a, b, c, d]).unwrap())
                .unwrap(),
        );
        const MSG: &[u8] = &[0xdb; 15 * 1024 * 1024];
        match proto_encode(MSG).unwrap() {
            ProtoEncodeResult::NeedPermit {
                permit_req,
                mut msg_payload,
            } => {
                match dec.decode(permit_req).unwrap() {
                    ProtoDecodeResult::PermitRequest(permit_len) => {
                        assert_eq!(15 * 1024 * 1024, permit_len);
                    }
                    _ => panic!(),
                }

                dec.sent_permit().unwrap();

                while msg_payload.len() > 1 {
                    assert_eq!(
                        ProtoDecodeResult::Idle,
                        dec.decode(msg_payload.remove(0)).unwrap(),
                    )
                }

                match dec.decode(msg_payload.remove(0)).unwrap() {
                    ProtoDecodeResult::Message(msg) => {
                        assert_eq!(MSG, msg);
                    }
                    _ => panic!(),
                }
            }
            _ => panic!(),
        }
    }

    #[test]
    fn proto_decode_bad_version() {
        let mut dec = ProtoDecoder::default();
        assert!(dec.decode(BackBuf::from_slice(b"hello").unwrap()).is_err());
    }
}
