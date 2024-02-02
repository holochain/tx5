//! Types associated with the Tx5 protocol.
//!
//! The tx5 protocol solves 2 problems at once:
//! - First, webrtc only supports messages up to 16K, so this lets us send
//!   bigger messages.
//! - Second, if we're sending bigger messages, we have to worry about the size
//!   in memory taken up by the receiving side. This protocol lets us request
//!   permits to send larger messages, which gives the receiving side a tool to
//!   be able to manage more open connections at the same time without worrying
//!   about the worst case memory usage of each connection all at the same time.

use crate::BackBuf;
use tx5_core::{Error, Result};

/// Tx5 protocol message payload max size.
pub(crate) const MAX_PAYLOAD: u32 = 0b00011111111111111111111111111111;

/// Protocol version 1 header.
pub(crate) const PROTO_VER_1: ProtoHeader =
    ProtoHeader::Version(1, [b't', b'x', b'5']);

/// 4 byte Tx5 protocol header.
///
/// The initial 3 bits representing values 0, 1, and 7 are reserved.
/// Decoders should error on receiving these values.
#[derive(Debug, PartialEq)]
pub(crate) enum ProtoHeader {
    /// This is a protocol version handshake.
    /// `3 bits == 2`
    /// `5 bits == version number (currently 1)`
    /// `24 bits == as bytes b"tx5"`
    Version(u8, [u8; 3]),

    /// The remainder of this message represents the entirety of a message.
    /// `3 bits == 3`
    /// `29 bits == the message size == full chunk size - 4`
    CompleteMessage(u32),

    /// The remainder of this message is a chunk of a multi-part message.
    /// `3 bits == 4`
    /// `29 bits == byte count included in this message chunk`
    MultipartMessage(u32),

    /// This is a request for a permit to send a message of size larger than
    /// a single chunk.
    /// `3 bits == 5`
    /// `29 bits == the size of the message`
    PermitRequest(u32),

    /// This is an authorization to proceed with sending payload chunks.
    /// `3 bits == 6`
    /// `29 bits == the size of the message`
    PermitGrant(u32),
}

impl ProtoHeader {
    /// Decode 4 bytes into a Tx5 protocol header.
    pub fn decode(a: u8, b: u8, c: u8, d: u8) -> Result<Self> {
        use bit_field::BitField;
        let r = u32::from_be_bytes([a, b, c, d]);
        match r.get_bits(29..) {
            2 => Ok(Self::Version(
                r.get_bits(24..29) as u8,
                [
                    r.get_bits(16..24) as u8,
                    r.get_bits(8..16) as u8,
                    r.get_bits(0..8) as u8,
                ],
            )),
            3 => Ok(Self::CompleteMessage(r.get_bits(..29))),
            4 => Ok(Self::MultipartMessage(r.get_bits(..29))),
            5 => Ok(Self::PermitRequest(r.get_bits(..29))),
            6 => Ok(Self::PermitGrant(r.get_bits(..29))),
            _ => Err(Error::id("ReservedHeaderBits")),
        }
    }

    /// Encode a Tx5 protocol header into canonical 4 bytes.
    pub fn encode(&self) -> Result<(u8, u8, u8, u8)> {
        use bit_field::BitField;
        let mut out: u32 = 0;
        match self {
            Self::Version(v, [a, b, c]) => {
                if *v > 31 {
                    return Err(Error::id("VersionOverflow"));
                }
                out.set_bits(29.., 2);
                out.set_bits(24..29, *v as u32);
                out.set_bits(16..24, *a as u32);
                out.set_bits(8..16, *b as u32);
                out.set_bits(0..8, *c as u32);
            }
            Self::CompleteMessage(s) => {
                if *s > MAX_PAYLOAD {
                    return Err(Error::id("SizeOverflow"));
                }
                out.set_bits(29.., 3);
                out.set_bits(..29, *s);
            }
            Self::MultipartMessage(s) => {
                if *s > MAX_PAYLOAD {
                    return Err(Error::id("SizeOverflow"));
                }
                out.set_bits(29.., 4);
                out.set_bits(..29, *s);
            }
            Self::PermitRequest(s) => {
                if *s > MAX_PAYLOAD {
                    return Err(Error::id("SizeOverflow"));
                }
                out.set_bits(29.., 5);
                out.set_bits(..29, *s);
            }
            Self::PermitGrant(s) => {
                if *s > MAX_PAYLOAD {
                    return Err(Error::id("SizeOverflow"));
                }
                out.set_bits(29.., 6);
                out.set_bits(..29, *s);
            }
        }
        let out = out.to_be_bytes();
        Ok((out[0], out[1], out[2], out[3]))
    }
}

/// Result of encoding a message into the Tx5 protocol.
pub(crate) enum ProtoEncodeResult {
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
pub(crate) fn proto_encode(data: &[u8]) -> Result<ProtoEncodeResult> {
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
        let permit_req = BackBuf::from_slice([a, b, c, d])?;

        let mut msg_payload = Vec::new();
        let mut cur = 0;

        while len - cur > 0 {
            let amt = std::cmp::min((16 * 1024) - 4, len - cur);

            let (a, b, c, d) =
                ProtoHeader::MultipartMessage(amt as u32).encode()?;

            let mut go_buf = tx5_go_pion::GoBuf::new()?;
            go_buf.reserve(amt + 4)?;
            go_buf.extend(&[a, b, c, d])?;
            go_buf.extend(&data[cur..cur + amt])?;

            msg_payload.push(BackBuf::from_raw(go_buf));

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
pub(crate) enum ProtoDecodeResult {
    /// Nothing needs to happen at the moment... continue receiving chunks.
    Idle,

    /// Received incoming message.
    Message(Vec<u8>),

    /// The remote node is requesting a permit to send us chunks of data.
    RemotePermitRequest(u32),

    /// The remote node has granted us a permit to send them chunks of data.
    RemotePermitGrant(u32),
}

#[derive(Clone, Copy, PartialEq)]
enum DecodeState {
    NeedVersion,
    Ready,
    /// The REMOTE requested a permit from US.
    /// Totally different from when we make a permit request of the remote : )
    RemoteAwaitingPermit(u32),
    ReceiveChunked,
}

/// Tx5 protocol decoder.
pub(crate) struct ProtoDecoder {
    state: DecodeState,
    want_size: usize,
    incoming: Vec<u8>,
    want_remote_grant: bool,
    did_error: bool,
    grant_permit: Option<tokio::sync::OwnedSemaphorePermit>,
    grant_notify: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Default for ProtoDecoder {
    fn default() -> Self {
        Self {
            state: DecodeState::NeedVersion,
            want_size: 0,
            incoming: Vec::new(),
            want_remote_grant: false,
            did_error: false,
            grant_permit: None,
            grant_notify: None,
        }
    }
}

impl ProtoDecoder {
    /// Notify the decoder that we sent the previously requested permit
    /// to the remote.
    pub fn sent_remote_permit_grant(
        &mut self,
        grant_permit: tokio::sync::OwnedSemaphorePermit,
    ) -> Result<()> {
        self.check_err()?;
        if let DecodeState::RemoteAwaitingPermit(permit_len) = self.state {
            self.state = DecodeState::ReceiveChunked;
            self.want_size = permit_len as usize;
            self.incoming.reserve(self.want_size);
            self.grant_permit = Some(grant_permit);
            Ok(())
        } else {
            self.did_error = true;
            Err(Error::id("InvalidStateToSendPermit"))
        }
    }

    /// Notify the decoder that we have requested a permit from the remote,
    /// so we should expect to receive a grant.
    pub fn sent_remote_permit_request(
        &mut self,
        grant_notify: Option<tokio::sync::oneshot::Sender<()>>,
    ) -> Result<()> {
        self.check_err()?;
        if self.want_remote_grant {
            self.did_error = true;
            Err(Error::id("InvalidDuplicatePermitRequest"))
        } else {
            self.want_remote_grant = true;
            self.grant_notify = grant_notify;
            Ok(())
        }
    }

    /// Process the next incoming chunk from the remote.
    pub fn decode(&mut self, chunk: BackBuf) -> Result<ProtoDecodeResult> {
        self.check_err()?;
        match self.priv_decode(chunk) {
            Ok(r) => Ok(r),
            Err(err) => {
                self.did_error = true;
                Err(err)
            }
        }
    }

    fn check_err(&self) -> Result<()> {
        if self.did_error {
            Err(Error::id("FnCallOnErroredDecoder"))
        } else {
            Ok(())
        }
    }

    fn priv_decode(&mut self, mut chunk: BackBuf) -> Result<ProtoDecodeResult> {
        chunk.imp.buf.access(|buf| {
            let buf = buf?;
            let len = buf.len();
            if len < 4 {
                return Err(Error::id("InvalidHeaderLen"));
            }

            match ProtoHeader::decode(buf[0], buf[1], buf[2], buf[3])? {
                ProtoHeader::Version(v, [a, b, c]) => {
                    if v != 1 || a != b't' || b != b'x' || c != b'5' {
                        return Err(Error::err(format!(
                            "invalid version v = {v}, tag = {}",
                            String::from_utf8_lossy(&[a, b, c][..]),
                        )));
                    }

                    if self.state == DecodeState::NeedVersion {
                        self.state = DecodeState::Ready;
                        Ok(ProtoDecodeResult::Idle)
                    } else {
                        Err(Error::id("RecvUnexpectedVersionMessage"))
                    }
                }
                ProtoHeader::CompleteMessage(msg_len) => {
                    if self.state == DecodeState::Ready {
                        if msg_len as usize != len - 4 {
                            return Err(Error::id("InvalidCompleteMessageLen"));
                        }

                        Ok(ProtoDecodeResult::Message(buf[4..].to_vec()))
                    } else {
                        Err(Error::id("RecvUnexpectedCompleteMessage"))
                    }
                }
                ProtoHeader::MultipartMessage(msg_len) => {
                    if self.state == DecodeState::ReceiveChunked {
                        if msg_len as usize != len - 4 || msg_len == 0 {
                            return Err(Error::id(
                                "InvalidMultipartMessageLen",
                            ));
                        }

                        if msg_len as usize + self.incoming.len()
                            > self.want_size
                        {
                            return Err(Error::id("ChunkTooLarge"));
                        }

                        self.incoming.extend_from_slice(&buf[4..]);

                        if self.incoming.len() == self.want_size {
                            drop(self.grant_permit.take());
                            self.state = DecodeState::Ready;
                            Ok(ProtoDecodeResult::Message(std::mem::take(
                                &mut self.incoming,
                            )))
                        } else {
                            Ok(ProtoDecodeResult::Idle)
                        }
                    } else {
                        Err(Error::id("RecvUnexpectedMultipartMessage"))
                    }
                }
                ProtoHeader::PermitRequest(permit_len) => {
                    if self.state == DecodeState::Ready {
                        self.state =
                            DecodeState::RemoteAwaitingPermit(permit_len);
                        Ok(ProtoDecodeResult::RemotePermitRequest(permit_len))
                    } else {
                        Err(Error::id("RecvUnexpectedPermitRequest"))
                    }
                }
                ProtoHeader::PermitGrant(permit_len) => {
                    if self.want_remote_grant {
                        self.want_remote_grant = false;
                        if let Some(grant_notify) = self.grant_notify.take() {
                            let _ = grant_notify.send(());
                        }
                        Ok(ProtoDecodeResult::RemotePermitGrant(permit_len))
                    } else {
                        Err(Error::id("RecvUnexpectedPermitGrant"))
                    }
                }
            }
        })
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn proto_header_encode_decode() {
        fn check(hdr: ProtoHeader) {
            let (a, b, c, d) = hdr.encode().unwrap();
            let res = ProtoHeader::decode(a, b, c, d).unwrap();
            assert_eq!(hdr, res);
        }

        for v in 0..32 {
            check(ProtoHeader::Version(v, [b't', b'x', b'5']));
        }

        for v in &[0, 42, 0b00011111111111111111111111111111] {
            check(ProtoHeader::CompleteMessage(*v));
            check(ProtoHeader::MultipartMessage(*v));
            check(ProtoHeader::PermitRequest(*v));
            check(ProtoHeader::PermitGrant(*v));
        }
    }

    #[test]
    fn proto_header_overflow() {
        assert!(ProtoHeader::Version(0b00100000, [b't', b'x', b'5'])
            .encode()
            .is_err());
        assert!(ProtoHeader::CompleteMessage(u32::MAX).encode().is_err());
        assert!(ProtoHeader::MultipartMessage(u32::MAX).encode().is_err());
        assert!(ProtoHeader::PermitRequest(u32::MAX).encode().is_err());
        assert!(ProtoHeader::PermitGrant(u32::MAX).encode().is_err());
    }

    #[test]
    fn proto_header_version_1() {
        const PROTO_VERSION_1: &[u8; 4] = &[
            0b01000001, // 010 for 3 bits == 2, 00001 for version #1
            b't', b'x', b'5',
        ];

        let (a, b, c, d) = PROTO_VER_1.encode().unwrap();

        assert_eq!(PROTO_VERSION_1[0], a);
        assert_eq!(PROTO_VERSION_1[1], b);
        assert_eq!(PROTO_VERSION_1[2], c);
        assert_eq!(PROTO_VERSION_1[3], d);
    }

    #[test]
    fn proto_decode_complete_msg() {
        let mut dec = ProtoDecoder::default();
        let (a, b, c, d) = PROTO_VER_1.encode().unwrap();
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
        use rand::Rng;
        let mut dec = ProtoDecoder::default();
        let (a, b, c, d) = PROTO_VER_1.encode().unwrap();
        assert_eq!(
            ProtoDecodeResult::Idle,
            dec.decode(BackBuf::from_slice(&[a, b, c, d]).unwrap())
                .unwrap(),
        );
        let mut msg = vec![0; 15 * 1024 * 1024];
        rand::thread_rng().fill(&mut msg[..]);
        match proto_encode(&msg).unwrap() {
            ProtoEncodeResult::NeedPermit {
                permit_req,
                mut msg_payload,
            } => {
                match dec.decode(permit_req).unwrap() {
                    ProtoDecodeResult::RemotePermitRequest(permit_len) => {
                        assert_eq!(15 * 1024 * 1024, permit_len);
                    }
                    _ => panic!(),
                }

                dec.sent_remote_permit_grant(
                    std::sync::Arc::new(tokio::sync::Semaphore::new(1))
                        .try_acquire_owned()
                        .unwrap(),
                )
                .unwrap();

                while msg_payload.len() > 1 {
                    assert_eq!(
                        ProtoDecodeResult::Idle,
                        dec.decode(msg_payload.remove(0)).unwrap(),
                    )
                }

                match dec.decode(msg_payload.remove(0)).unwrap() {
                    ProtoDecodeResult::Message(msg_res) => {
                        assert_eq!(msg, msg_res);
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

    #[test]
    fn proto_decode_no_duplicate_permit_requests() {
        let mut dec = ProtoDecoder::default();
        let (a, b, c, d) = PROTO_VER_1.encode().unwrap();
        assert_eq!(
            ProtoDecodeResult::Idle,
            dec.decode(BackBuf::from_slice(&[a, b, c, d]).unwrap())
                .unwrap(),
        );
        dec.sent_remote_permit_request(None).unwrap();
        assert!(dec.sent_remote_permit_request(None).is_err());
    }

    #[test]
    fn proto_decode_grant_during_multipart() {
        use rand::Rng;
        let mut dec = ProtoDecoder::default();
        let (a, b, c, d) = PROTO_VER_1.encode().unwrap();
        assert_eq!(
            ProtoDecodeResult::Idle,
            dec.decode(BackBuf::from_slice(&[a, b, c, d]).unwrap())
                .unwrap(),
        );

        dec.sent_remote_permit_request(None).unwrap();

        let mut msg = vec![0; 17 * 1024];
        rand::thread_rng().fill(&mut msg[..]);
        match proto_encode(&msg).unwrap() {
            ProtoEncodeResult::NeedPermit {
                permit_req,
                mut msg_payload,
            } => {
                match dec.decode(permit_req).unwrap() {
                    ProtoDecodeResult::RemotePermitRequest(permit_len) => {
                        assert_eq!(17 * 1024, permit_len);
                    }
                    _ => panic!(),
                }

                dec.sent_remote_permit_grant(
                    std::sync::Arc::new(tokio::sync::Semaphore::new(1))
                        .try_acquire_owned()
                        .unwrap(),
                )
                .unwrap();

                assert_eq!(2, msg_payload.len());

                assert_eq!(
                    ProtoDecodeResult::Idle,
                    dec.decode(msg_payload.remove(0)).unwrap(),
                );

                let (a, b, c, d) =
                    ProtoHeader::PermitGrant(18 * 1024).encode().unwrap();
                assert_eq!(
                    ProtoDecodeResult::RemotePermitGrant(18 * 1024),
                    dec.decode(BackBuf::from_slice(&[a, b, c, d]).unwrap())
                        .unwrap(),
                );

                match dec.decode(msg_payload.remove(0)).unwrap() {
                    ProtoDecodeResult::Message(msg_res) => {
                        assert_eq!(msg, msg_res);
                    }
                    _ => panic!(),
                }
            }
            _ => panic!(),
        }
    }
}
