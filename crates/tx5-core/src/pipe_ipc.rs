//! Types for dealing with the tx5-pipe ipc protocol.

use asv::*;
use bytes::Buf;

/// Request types that can be received by a Tx5Pipe server from a client.
pub enum Tx5PipeRequest {
    /// A request to register as addressable with a signal server.
    SigReg {
        /// Command identifier.
        cmd_id: String,

        /// Signal url.
        sig_url: String,
    },

    /// A request to make this node discoverable on a bootstrap server.
    BootReg {
        /// Command identifier.
        cmd_id: String,

        /// Bootstrap server url.
        boot_url: String,

        /// Application hash.
        app_hash: [u8; 32],

        /// Signal client url.
        /// (None to un-register).
        cli_url: Option<String>,

        /// Bootstrap meta data.
        data: Box<dyn bytes::Buf + Send>,
    },

    /// Query a bootstrap server for peers on a given app hash.
    BootQuery {
        /// Command identifier.
        cmd_id: String,

        /// Bootstrap server url.
        boot_url: String,

        /// Application hash.
        app_hash: [u8; 32],
    },

    /// A request to send a message to a remote peer.
    Send {
        /// Command identifier.
        cmd_id: String,

        /// Remote url.
        rem_url: String,

        /// Message data.
        data: Box<dyn bytes::Buf + Send>,
    },
}

impl Tx5PipeRequest {
    /// If this response type contains a cmd_id, return it.
    pub fn get_cmd_id(&self) -> String {
        match self {
            Self::SigReg { cmd_id, .. }
            | Self::BootReg { cmd_id, .. }
            | Self::BootQuery { cmd_id, .. }
            | Self::Send { cmd_id, .. } => cmd_id.clone(),
        }
    }

    /// Register to be addressable with a signal server.
    pub fn sig_reg(
        enc: &mut AsvEncoder,
        cmd_id: String,
        sig_url: String,
    ) -> std::io::Result<()> {
        enc.field(&b"sig_reg"[..]);
        enc.field(cmd_id.into_bytes());
        enc.field(sig_url.into_bytes());
        enc.finish_row();
        Ok(())
    }

    /// A request to make this node discoverable on a bootstrap server.
    pub fn boot_reg(
        enc: &mut AsvEncoder,
        cmd_id: String,
        boot_url: String,
        app_hash: [u8; 32],
        cli_url: Option<String>,
        data: Box<dyn Buf + Send>,
    ) -> std::io::Result<()> {
        if data.remaining() > 512 {
            return Err(crate::Error::id("BootDataOver512B"));
        }
        let b64 = base64::encode(app_hash);
        enc.field(&b"boot_reg"[..]);
        enc.field(cmd_id.into_bytes());
        enc.field(boot_url.into_bytes());
        enc.field(b64.into_bytes());
        enc.field(cli_url.unwrap_or_default().into_bytes());
        enc.binary_boxed(data);
        enc.finish_row();
        Ok(())
    }

    /// Query a bootstrap server for peers on a given app hash.
    pub fn boot_query(
        enc: &mut AsvEncoder,
        cmd_id: String,
        boot_url: String,
        app_hash: [u8; 32],
    ) -> std::io::Result<()> {
        let b64 = base64::encode(app_hash);
        enc.field(&b"boot_query"[..]);
        enc.field(cmd_id.into_bytes());
        enc.field(boot_url.into_bytes());
        enc.field(b64.into_bytes());
        enc.finish_row();
        Ok(())
    }

    /// Send a message to a remote.
    pub fn send(
        enc: &mut AsvEncoder,
        cmd_id: String,
        rem_url: String,
        data: Box<dyn Buf + Send>,
    ) -> std::io::Result<()> {
        if data.remaining() > 16 * 1024 {
            return Err(crate::Error::id("MsgOver16KiB"));
        }
        enc.field(&b"send"[..]);
        enc.field(cmd_id.into_bytes());
        enc.field(rem_url.into_bytes());
        enc.binary_boxed(data);
        enc.finish_row();
        Ok(())
    }

    /// Encode this instance in the tx5 pipe ipc protocol.
    pub fn encode(self, enc: &mut AsvEncoder) -> std::io::Result<()> {
        match self {
            Self::SigReg { cmd_id, sig_url } => {
                Self::sig_reg(enc, cmd_id, sig_url)
            }
            Self::BootReg {
                cmd_id,
                boot_url,
                app_hash,
                cli_url,
                data,
            } => Self::boot_reg(enc, cmd_id, boot_url, app_hash, cli_url, data),
            Self::BootQuery {
                cmd_id,
                boot_url,
                app_hash,
            } => Self::boot_query(enc, cmd_id, boot_url, app_hash),
            Self::Send {
                cmd_id,
                rem_url,
                data,
            } => Self::send(enc, cmd_id, rem_url, data),
        }
    }

    /// Decode a [Tx5PipeRequest] instance from an already parsed
    /// tx5 pipe ipc protocol message. See [asv::AsvParser].
    pub fn decode(mut fields: Vec<AsvBuffer>) -> std::io::Result<Self> {
        if fields.is_empty() {
            return Err(crate::Error::id("EmptyFieldList"));
        }
        match fields.remove(0).into_string()?.as_str() {
            "sig_reg" => {
                if fields.len() != 2 {
                    return Err(crate::Error::id("InvalidArgs"));
                }
                let cmd_id = fields.remove(0).into_string()?;
                let sig_url = fields.remove(0).into_string()?;
                Ok(Self::SigReg { cmd_id, sig_url })
            }
            "boot_reg" => {
                if fields.len() != 5 {
                    return Err(crate::Error::id("InvalidArgs"));
                }
                let cmd_id = fields.remove(0).into_string()?;
                let boot_url = fields.remove(0).into_string()?;
                let data = fields.remove(0).into_vec();
                let app_hash_unsized =
                    base64::decode(data).map_err(crate::Error::err)?;
                if app_hash_unsized.len() != 32 {
                    return Err(crate::Error::id("InvalidAppHashSize"));
                }
                let mut app_hash = [0; 32];
                app_hash.copy_from_slice(&app_hash_unsized);
                let data = fields.remove(0);
                let cli_url = if data.has_remaining() {
                    Some(data.into_string()?)
                } else {
                    None
                };
                let data = fields.remove(0);
                if data.remaining() > 512 {
                    return Err(crate::Error::id("BootDataOver512B"));
                }
                let data = Box::new(data);
                Ok(Self::BootReg {
                    cmd_id,
                    boot_url,
                    app_hash,
                    cli_url,
                    data,
                })
            }
            "boot_query" => {
                if fields.len() != 3 {
                    return Err(crate::Error::id("InvalidArgs"));
                }
                let cmd_id = fields.remove(0).into_string()?;
                let boot_url = fields.remove(0).into_string()?;
                let data = fields.remove(0).into_vec();
                let app_hash_unsized =
                    base64::decode(data).map_err(crate::Error::err)?;
                if app_hash_unsized.len() != 32 {
                    return Err(crate::Error::id("InvalidAppHashSize"));
                }
                let mut app_hash = [0; 32];
                app_hash.copy_from_slice(&app_hash_unsized);
                Ok(Self::BootQuery {
                    cmd_id,
                    boot_url,
                    app_hash,
                })
            }
            "send" => {
                if fields.len() != 3 {
                    return Err(crate::Error::id("InvalidArgs"));
                }
                let cmd_id = fields.remove(0).into_string()?;
                let rem_url = fields.remove(0).into_string()?;
                let data = fields.remove(0);
                if data.remaining() > 16 * 1024 {
                    return Err(crate::Error::id("MsgOver16KiB"));
                }
                let data = Box::new(data);
                Ok(Self::Send {
                    cmd_id,
                    rem_url,
                    data,
                })
            }
            oth => Err(crate::Error::err(format!("invalid cmd: {oth}"))),
        }
    }
}

/// Response types that can be received by a Tx5Pipe client from a server.
pub enum Tx5PipeResponse {
    /// Unsolicited help info.
    Help {
        /// Server version.
        version: String,

        /// Help info.
        info: String,
    },

    /// An error response.
    Error {
        /// Command identifier.
        cmd_id: String,

        /// Error code.
        code: u32,

        /// Error text.
        text: String,
    },

    /// An ok response to a sig_reg request.
    SigRegOk {
        /// Command identifier.
        cmd_id: String,

        /// Client Url.
        cli_url: String,
    },

    /// An ok response to a boot_reg request.
    BootRegOk {
        /// Command identifier.
        cmd_id: String,
    },

    /// An ok response to a boot_query request.
    BootQueryOk {
        /// Command identifier.
        cmd_id: String,
    },

    /// An ok response to a send request.
    SendOk {
        /// Command identifier.
        cmd_id: String,
    },

    /// Receive a message from a remote.
    Recv {
        /// Remote url.
        rem_url: String,

        /// Message data.
        data: Box<dyn bytes::Buf + Send>,
    },

    /// Receive a response item for a boot_query.
    BootQueryResp {
        /// Command identifier.
        cmd_id: String,

        /// Remote pub_key.
        rem_pub_key: [u8; 32],

        /// Remote url.
        rem_url: Option<String>,

        /// Expiration unix epoch seconds.
        expires_at_s: u64,

        /// Bootstrap meta data.
        data: Box<dyn bytes::Buf + Send>,
    },
}

impl Tx5PipeResponse {
    /// If this response type contains a cmd_id, return it.
    pub fn get_cmd_id(&self) -> Option<String> {
        match self {
            Self::Help { .. } | Self::Recv { .. } => None,
            Self::Error { cmd_id, .. }
            | Self::SigRegOk { cmd_id, .. }
            | Self::BootRegOk { cmd_id, .. }
            | Self::BootQueryOk { cmd_id, .. }
            | Self::SendOk { cmd_id, .. }
            | Self::BootQueryResp { cmd_id, .. } => Some(cmd_id.clone()),
        }
    }

    /// Send unsolicited help information.
    pub fn help(
        enc: &mut AsvEncoder,
        version: String,
        info: String,
    ) -> std::io::Result<()> {
        enc.field(&b"@help"[..]);
        enc.field(version.into_bytes());
        enc.field(info.into_bytes());
        enc.finish_row();
        Ok(())
    }

    /// If you need to send an error response to a command.
    pub fn error(
        enc: &mut AsvEncoder,
        cmd_id: String,
        code: u32,
        text: String,
    ) -> std::io::Result<()> {
        let code = format!("{code}");
        enc.field(&b"@error"[..]);
        enc.field(cmd_id.into_bytes());
        enc.field(code.into_bytes());
        enc.field(text.into_bytes());
        enc.finish_row();
        Ok(())
    }

    /// Okay response to a sig_reg request.
    pub fn sig_reg_ok(
        enc: &mut AsvEncoder,
        cmd_id: String,
        cli_url: String,
    ) -> std::io::Result<()> {
        enc.field(&b"@sig_reg_ok"[..]);
        enc.field(cmd_id.into_bytes());
        enc.field(cli_url.into_bytes());
        enc.finish_row();
        Ok(())
    }

    /// Okay response to a send request.
    pub fn send_ok(
        enc: &mut AsvEncoder,
        cmd_id: String,
    ) -> std::io::Result<()> {
        enc.field(&b"@send_ok"[..]);
        enc.field(cmd_id.into_bytes());
        enc.finish_row();
        Ok(())
    }

    /// Okay response to an boot_reg request.
    pub fn boot_reg_ok(
        enc: &mut AsvEncoder,
        cmd_id: String,
    ) -> std::io::Result<()> {
        enc.field(&b"@boot_reg_ok"[..]);
        enc.field(cmd_id.into_bytes());
        enc.finish_row();
        Ok(())
    }

    /// Okay response to an boot_query request.
    pub fn boot_query_ok(
        enc: &mut AsvEncoder,
        cmd_id: String,
    ) -> std::io::Result<()> {
        enc.field(&b"@boot_query_ok"[..]);
        enc.field(cmd_id.into_bytes());
        enc.finish_row();
        Ok(())
    }

    /// Receive data from a remote peer.
    pub fn recv(
        enc: &mut AsvEncoder,
        rem_url: String,
        data: Box<dyn Buf + Send>,
    ) -> std::io::Result<()> {
        enc.field(&b"@recv"[..]);
        enc.field(rem_url.into_bytes());
        enc.binary_boxed(data);
        enc.finish_row();
        Ok(())
    }

    /// Receive a boot notice.
    pub fn boot_query_resp(
        enc: &mut AsvEncoder,
        cmd_id: String,
        rem_pub_key: [u8; 32],
        rem_url: Option<String>,
        expires_at_s: u64,
        data: Box<dyn Buf + Send>,
    ) -> std::io::Result<()> {
        if data.remaining() > 512 {
            return Err(crate::Error::id("BootDataOver512B"));
        }
        let b64 = base64::encode(rem_pub_key);
        enc.field(&b"@boot_query_resp"[..]);
        enc.field(cmd_id.into_bytes());
        enc.field(b64.into_bytes());
        enc.field(rem_url.unwrap_or_default().into_bytes());
        enc.field(format!("{expires_at_s}").into_bytes());
        enc.binary_boxed(data);
        enc.finish_row();
        Ok(())
    }

    /// Encode this instance in the tx5 pipe ipc protocol.
    pub fn encode(self, enc: &mut AsvEncoder) -> std::io::Result<()> {
        match self {
            Self::Help { version, info } => Self::help(enc, version, info),
            Self::Error { cmd_id, code, text } => {
                Self::error(enc, cmd_id, code, text)
            }
            Self::SigRegOk { cmd_id, cli_url } => {
                Self::sig_reg_ok(enc, cmd_id, cli_url)
            }
            Self::BootRegOk { cmd_id } => Self::boot_reg_ok(enc, cmd_id),
            Self::BootQueryOk { cmd_id } => Self::boot_query_ok(enc, cmd_id),
            Self::SendOk { cmd_id } => Self::send_ok(enc, cmd_id),
            Self::Recv { rem_url, data } => Self::recv(enc, rem_url, data),
            Self::BootQueryResp {
                cmd_id,
                rem_pub_key,
                rem_url,
                expires_at_s,
                data,
            } => Self::boot_query_resp(
                enc,
                cmd_id,
                rem_pub_key,
                rem_url,
                expires_at_s,
                data,
            ),
        }
    }

    /// Decode a [Tx5PipeResponse] instance from an already parsed
    /// tx5 pipe ipc protocol message. See [asv::AsvParser].
    pub fn decode(mut fields: Vec<AsvBuffer>) -> std::io::Result<Self> {
        if fields.is_empty() {
            return Err(crate::Error::id("EmptyFieldList"));
        }
        match fields.remove(0).into_string()?.as_str() {
            "@help" => {
                if fields.len() != 2 {
                    return Err(crate::Error::id("InvalidArgs"));
                }
                let version = fields.remove(0).into_string()?;
                let info = fields.remove(0).into_string()?;
                Ok(Self::Help { version, info })
            }
            "@error" => {
                if fields.len() != 3 {
                    return Err(crate::Error::id("InvalidArgs"));
                }
                let cmd_id = fields.remove(0).into_string()?;
                let code = fields.remove(0).into_string()?;
                let text = fields.remove(0).into_string()?;
                Ok(Self::Error {
                    cmd_id,
                    code: code.parse().map_err(crate::Error::err)?,
                    text,
                })
            }
            "@sig_reg_ok" => {
                if fields.len() != 2 {
                    return Err(crate::Error::id("InvalidArgs"));
                }
                let cmd_id = fields.remove(0).into_string()?;
                let cli_url = fields.remove(0).into_string()?;
                Ok(Self::SigRegOk { cmd_id, cli_url })
            }
            "@boot_reg_ok" => {
                if fields.len() != 1 {
                    return Err(crate::Error::id("InvalidArgs"));
                }
                let cmd_id = fields.remove(0).into_string()?;
                Ok(Self::BootRegOk { cmd_id })
            }
            "@boot_query_ok" => {
                if fields.len() != 1 {
                    return Err(crate::Error::id("InvalidArgs"));
                }
                let cmd_id = fields.remove(0).into_string()?;
                Ok(Self::BootQueryOk { cmd_id })
            }
            "@send_ok" => {
                if fields.len() != 1 {
                    return Err(crate::Error::id("InvalidArgs"));
                }
                let cmd_id = fields.remove(0).into_string()?;
                Ok(Self::SendOk { cmd_id })
            }
            "@recv" => {
                if fields.len() != 2 {
                    return Err(crate::Error::id("InvalidArgs"));
                }
                let rem_url = fields.remove(0).into_string()?;
                let data = fields.remove(0);
                if data.remaining() > 16 * 1024 {
                    return Err(crate::Error::id("MsgOver16KiB"));
                }
                let data = Box::new(data);
                Ok(Self::Recv { rem_url, data })
            }
            "@boot_query_resp" => {
                if fields.len() != 5 {
                    return Err(crate::Error::id("InvalidArgs"));
                }

                let cmd_id = fields.remove(0).into_string()?;
                let data = fields.remove(0).into_vec();
                let pk_unsized =
                    base64::decode(data).map_err(crate::Error::err)?;
                if pk_unsized.len() != 32 {
                    return Err(crate::Error::id("InvalidRemPubKeySize"));
                }
                let mut rem_pub_key = [0; 32];
                rem_pub_key.copy_from_slice(&pk_unsized);
                let data = fields.remove(0);
                let rem_url = if data.has_remaining() {
                    Some(data.into_string()?)
                } else {
                    None
                };
                let expires_at_s = fields.remove(0).into_string()?;
                let data = fields.remove(0);
                if data.remaining() > 512 {
                    return Err(crate::Error::id("BootDataOver512B"));
                }
                let data = Box::new(data);
                Ok(Self::BootQueryResp {
                    cmd_id,
                    rem_pub_key,
                    rem_url,
                    expires_at_s: expires_at_s
                        .parse()
                        .map_err(crate::Error::err)?,
                    data,
                })
            }
            oth => Err(crate::Error::err(format!("invalid cmd: {oth}"))),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    const REQ_RES_FIX: &[&[&str]] = &[
        &["sig_reg", "test10", "wss://yada"],
        &[
            "boot_reg",
            "test20",
            "https://yada",
            "Ov8rjjg6jzhf7yUlp4S9Q1L9s9wZhaKJGe2mB4pax0k=",
            "wss://yada",
            "yada",
        ],
        &[
            "boot_query",
            "test30",
            "https://yada",
            "Ov8rjjg6jzhf7yUlp4S9Q1L9s9wZhaKJGe2mB4pax0k=",
        ],
        &["send", "test40", "wss://yada", "yada"],
        &["@help", "test50", "yada\nmultiline"],
        &["@error", "test60", "42", "yada"],
        &["@sig_reg_ok", "test70", "yada"],
        &["@boot_reg_ok", "test80"],
        &["@boot_query_ok", "test90"],
        &["@send_ok", "testA0"],
        &["@recv", "testB0", "yada"],
        &[
            "@boot_query_resp",
            "testC0",
            "Ov8rjjg6jzhf7yUlp4S9Q1L9s9wZhaKJGe2mB4pax0k=",
            "wss://yada",
            "42",
            "yada",
        ],
        &[
            "@boot_query_resp",
            "testD0",
            "Ov8rjjg6jzhf7yUlp4S9Q1L9s9wZhaKJGe2mB4pax0k=",
            "wss://yada",
            "42",
            "",
        ],
    ];

    #[test]
    fn request_response() {
        for field_list in REQ_RES_FIX {
            let is_resp = field_list[0].starts_with("@");
            let mut by_hand = Vec::new();
            for field in *field_list {
                by_hand.extend_from_slice(
                    format!("`{}|", field.as_bytes().len()).as_bytes(),
                );
                by_hand.extend_from_slice(field.as_bytes());
                by_hand.extend_from_slice(b"` ");
            }
            by_hand.push(b'\n');
            let mut p = asv::AsvParser::default().parse(by_hand).unwrap();
            assert_eq!(1, p.len());
            let p = p.remove(0);
            let mut enc = asv::AsvEncoder::default();
            if is_resp {
                Tx5PipeResponse::decode(p)
                    .unwrap()
                    .encode(&mut enc)
                    .unwrap();
            } else {
                Tx5PipeRequest::decode(p).unwrap().encode(&mut enc).unwrap();
            }
            let enc = enc.drain().into_vec();
            let mut p = asv::AsvParser::default().parse(enc).unwrap();
            assert_eq!(1, p.len());
            let p = p.remove(0);
            let mut p = p.into_iter();
            for field in *field_list {
                let actual = p.next().unwrap().into_string().unwrap();
                assert_eq!(*field, actual);
            }
        }
    }
}
