//! Types for dealing with the tx5-pipe ipc protocol.

use asv::asv_encoder::*;
use bytes::Buf;

/// Helper class to translate fields and binary data into ASV format rows.
pub struct Tx5PipeWriteRow<'lt, W: std::io::Write> {
    w: &'lt mut W,
    buf: Vec<u8>,
    is_first: bool,
}

impl<'lt, W: std::io::Write> Drop for Tx5PipeWriteRow<'lt, W> {
    fn drop(&mut self) {
        let _ = self.priv_finish();
    }
}

impl<'lt, W: std::io::Write> Tx5PipeWriteRow<'lt, W> {
    /// Construct a new row write helper.
    pub fn new(w: &'lt mut W) -> Self {
        Self {
            w,
            buf: Vec::new(),
            is_first: true,
        }
    }

    /// Write an explicitly binary field to the row.
    pub fn write_bin(
        mut self,
        mut bin: Box<dyn Buf + Send>,
    ) -> std::io::Result<Self> {
        self.check_first();
        self.buf
            .extend_from_slice(&write_bin_header(bin.remaining()));
        self.write_buf()?;
        while bin.has_remaining() {
            let c = bin.chunk();
            self.w.write_all(c)?;
            bin.advance(c.len());
        }
        self.buf.extend_from_slice(BIN_FOOTER);
        Ok(self)
    }

    /// Write a field to the row.
    /// Will use autopilot for determining field format.
    pub fn write_field(mut self, field: &[u8]) -> std::io::Result<Self> {
        self.check_first();
        match get_best_fmt(field) {
            BestFmt::Bin => {
                self.buf.extend_from_slice(&write_bin_header(field.len()));
                self.write_buf()?;
                self.w.write_all(field)?;
                self.buf.extend_from_slice(BIN_FOOTER);
            }
            BestFmt::Raw => {
                self.buf.extend_from_slice(&write_raw_field(field));
            }
            BestFmt::Quote => {
                self.buf.extend_from_slice(&write_quote_field(field));
            }
        }
        Ok(self)
    }

    /// Finish the row.
    pub fn finish(mut self) -> std::io::Result<()> {
        self.priv_finish()
    }

    fn check_first(&mut self) {
        if self.is_first {
            self.is_first = false;
        } else {
            self.buf.push(b' ');
        }
    }

    fn priv_finish(&mut self) -> std::io::Result<()> {
        self.buf.push(b'\n');
        self.write_buf()
    }

    fn write_buf(&mut self) -> std::io::Result<()> {
        self.w.write_all(&self.buf)?;
        self.buf.clear();
        Ok(())
    }
}

/// Request types that can be received by a Tx5Pipe server from a client.
pub enum Tx5PipeRequest {
    /// Register an application hash.
    AppReg {
        /// Command identifier.
        cmd_id: String,

        /// Application hash.
        app_hash: [u8; 32],
    },

    /// A request to register as addressable with a signal server.
    SigReg {
        /// Command identifier.
        cmd_id: String,

        /// Signal Url.
        sig_url: String,
    },

    /// A request to make this node discoverable on a bootstrap server.
    BootReg {
        /// Command identifier.
        cmd_id: String,

        /// Bootstrap server url.
        boot_url: String,

        /// Bootstrap meta data.
        data: Box<dyn bytes::Buf + Send>,
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
            Self::AppReg { cmd_id, .. }
            | Self::SigReg { cmd_id, .. }
            | Self::BootReg { cmd_id, .. }
            | Self::Send { cmd_id, .. } => cmd_id.clone(),
        }
    }

    /// Register an application hash.
    pub fn tx5_app_reg<W: std::io::Write>(
        w: &mut W,
        cmd_id: &str,
        app_hash: &[u8; 32],
    ) -> std::io::Result<()> {
        let b64 = base64::encode(app_hash);
        Tx5PipeWriteRow::new(w)
            .write_field(b"app_reg")?
            .write_field(cmd_id.as_bytes())?
            .write_field(b64.as_bytes())?
            .finish()
    }

    /// Register to be addressable with a signal server.
    pub fn tx5_sig_reg<W: std::io::Write>(
        w: &mut W,
        cmd_id: &str,
        sig_url: &str,
    ) -> std::io::Result<()> {
        Tx5PipeWriteRow::new(w)
            .write_field(b"sig_reg")?
            .write_field(cmd_id.as_bytes())?
            .write_field(sig_url.as_bytes())?
            .finish()
    }

    /// A request to make this node discoverable on a bootstrap server.
    pub fn tx5_boot_reg<W: std::io::Write>(
        w: &mut W,
        cmd_id: &str,
        boot_url: &str,
        data: Box<dyn Buf + Send>,
    ) -> std::io::Result<()> {
        if data.remaining() > 512 {
            return Err(crate::Error::id("BootDataOver512B"));
        }
        Tx5PipeWriteRow::new(w)
            .write_field(b"boot_reg")?
            .write_field(cmd_id.as_bytes())?
            .write_field(boot_url.as_bytes())?
            .write_bin(data)?
            .finish()
    }

    /// Send a message to a remote.
    pub fn tx5_send<W: std::io::Write>(
        w: &mut W,
        cmd_id: &str,
        rem_url: &str,
        data: Box<dyn Buf + Send>,
    ) -> std::io::Result<()> {
        if data.remaining() > 16 * 1024 {
            return Err(crate::Error::id("MsgOver16KiB"));
        }
        Tx5PipeWriteRow::new(w)
            .write_field(b"send")?
            .write_field(cmd_id.as_bytes())?
            .write_field(rem_url.as_bytes())?
            .write_bin(data)?
            .finish()
    }

    /// Encode this instance in the tx5 pipe ipc protocol.
    pub fn encode<W: std::io::Write>(self, w: &mut W) -> std::io::Result<()> {
        match self {
            Self::AppReg { cmd_id, app_hash } => {
                Self::tx5_app_reg(w, &cmd_id, &app_hash)
            }
            Self::SigReg { cmd_id, sig_url } => {
                Self::tx5_sig_reg(w, &cmd_id, &sig_url)
            }
            Self::BootReg {
                cmd_id,
                boot_url,
                data,
            } => Self::tx5_boot_reg(w, &cmd_id, &boot_url, data),
            Self::Send {
                cmd_id,
                rem_url,
                data,
            } => Self::tx5_send(w, &cmd_id, &rem_url, data),
        }
    }

    /// Decode a [Tx5PipeRequest] instance from an already parsed
    /// tx5 pipe ipc protocol message. See [asv::AsvParse].
    pub fn decode(mut fields: Vec<Vec<u8>>) -> std::io::Result<Self> {
        if fields.is_empty() {
            return Err(crate::Error::id("EmptyFieldList"));
        }
        match fields.remove(0).as_slice() {
            b"app_reg" => {
                if fields.len() != 2 {
                    return Err(crate::Error::id("InvalidArgs"));
                }
                let cmd_id =
                    String::from_utf8_lossy(&fields.remove(0)).to_string();
                let data = fields.remove(0);
                let app_hash_unsized =
                    base64::decode(data).map_err(crate::Error::err)?;
                if app_hash_unsized.len() != 32 {
                    return Err(crate::Error::id("InvalidAppHashSize"));
                }
                let mut app_hash = [0; 32];
                app_hash.copy_from_slice(&app_hash_unsized);
                Ok(Self::AppReg { cmd_id, app_hash })
            }
            b"sig_reg" => {
                if fields.len() != 2 {
                    return Err(crate::Error::id("InvalidArgs"));
                }
                let cmd_id =
                    String::from_utf8_lossy(&fields.remove(0)).to_string();
                let sig_url =
                    String::from_utf8_lossy(&fields.remove(0)).to_string();
                Ok(Self::SigReg { cmd_id, sig_url })
            }
            b"boot_reg" => {
                if fields.len() != 3 {
                    return Err(crate::Error::id("InvalidArgs"));
                }
                let cmd_id =
                    String::from_utf8_lossy(&fields.remove(0)).to_string();
                let boot_url =
                    String::from_utf8_lossy(&fields.remove(0)).to_string();
                let data = fields.remove(0);
                if data.len() > 512 {
                    return Err(crate::Error::id("BootDataOver512B"));
                }
                let data = Box::new(std::io::Cursor::new(data));
                Ok(Self::BootReg {
                    cmd_id,
                    boot_url,
                    data,
                })
            }
            b"send" => {
                if fields.len() != 3 {
                    return Err(crate::Error::id("InvalidArgs"));
                }
                let cmd_id =
                    String::from_utf8_lossy(&fields.remove(0)).to_string();
                let rem_url =
                    String::from_utf8_lossy(&fields.remove(0)).to_string();
                let data = fields.remove(0);
                if data.len() > 16 * 1024 {
                    return Err(crate::Error::id("MsgOver16KiB"));
                }
                let data = Box::new(std::io::Cursor::new(data));
                Ok(Self::Send {
                    cmd_id,
                    rem_url,
                    data,
                })
            }
            oth => Err(crate::Error::err(format!(
                "invalid cmd: {}",
                String::from_utf8_lossy(oth)
            ))),
        }
    }
}

/// Response types that can be received by a Tx5Pipe client from a server.
pub enum Tx5PipeResponse {
    /// Unsolicited help info.
    Tx5PipeHelp {
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

    /// An ok response to an app_reg request.
    AppRegOk {
        /// Command identifier.
        cmd_id: String,
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

    /// Receive a boot notice.
    BootRecv {
        /// Remote pub_key.
        rem_pub_key: [u8; 32],

        /// Remote url.
        rem_url: Option<String>,

        /// Bootstrap meta data.
        data: Box<dyn bytes::Buf + Send>,
    },
}

impl Tx5PipeResponse {
    /// If this response type contains a cmd_id, return it.
    pub fn get_cmd_id(&self) -> Option<String> {
        match self {
            Self::Tx5PipeHelp { .. }
            | Self::Recv { .. }
            | Self::BootRecv { .. } => None,
            Self::Error { cmd_id, .. }
            | Self::AppRegOk { cmd_id }
            | Self::SigRegOk { cmd_id, .. }
            | Self::BootRegOk { cmd_id, .. }
            | Self::SendOk { cmd_id, .. } => Some(cmd_id.clone()),
        }
    }

    /// Send unsolicited help information.
    pub fn tx5_pipe_help<W: std::io::Write>(
        w: &mut W,
        version: &str,
        info: &str,
    ) -> std::io::Result<()> {
        Tx5PipeWriteRow::new(w)
            .write_field(b"@help")?
            .write_field(version.as_bytes())?
            .write_field(info.as_bytes())?
            .finish()
    }

    /// If you need to send an error response to a command.
    pub fn tx5_error<W: std::io::Write>(
        w: &mut W,
        cmd_id: &str,
        code: u32,
        text: &str,
    ) -> std::io::Result<()> {
        let code = format!("{code}");
        Tx5PipeWriteRow::new(w)
            .write_field(b"@error")?
            .write_field(cmd_id.as_bytes())?
            .write_field(code.as_bytes())?
            .write_field(text.as_bytes())?
            .finish()
    }

    /// Okay response to a sig_reg request.
    pub fn tx5_sig_reg_ok<W: std::io::Write>(
        w: &mut W,
        cmd_id: &str,
        cli_url: &str,
    ) -> std::io::Result<()> {
        Tx5PipeWriteRow::new(w)
            .write_field(b"@sig_reg_ok")?
            .write_field(cmd_id.as_bytes())?
            .write_field(cli_url.as_bytes())?
            .finish()
    }

    /// Okay response to an app_reg request.
    pub fn tx5_app_reg_ok<W: std::io::Write>(
        w: &mut W,
        cmd_id: &str,
    ) -> std::io::Result<()> {
        Tx5PipeWriteRow::new(w)
            .write_field(b"@app_reg_ok")?
            .write_field(cmd_id.as_bytes())?
            .finish()
    }

    /// Okay response to a send request.
    pub fn tx5_send_ok<W: std::io::Write>(
        w: &mut W,
        cmd_id: &str,
    ) -> std::io::Result<()> {
        Tx5PipeWriteRow::new(w)
            .write_field(b"@send_ok")?
            .write_field(cmd_id.as_bytes())?
            .finish()
    }

    /// Okay response to an boot_reg request.
    pub fn tx5_boot_reg_ok<W: std::io::Write>(
        w: &mut W,
        cmd_id: &str,
    ) -> std::io::Result<()> {
        Tx5PipeWriteRow::new(w)
            .write_field(b"@boot_reg_ok")?
            .write_field(cmd_id.as_bytes())?
            .finish()
    }

    /// Receive data from a remote peer.
    pub fn tx5_recv<W: std::io::Write>(
        w: &mut W,
        rem_url: &str,
        data: Box<dyn Buf + Send>,
    ) -> std::io::Result<()> {
        Tx5PipeWriteRow::new(w)
            .write_field(b"@recv")?
            .write_field(rem_url.as_bytes())?
            .write_bin(data)?
            .finish()
    }

    /// Receive a boot notice.
    pub fn tx5_boot_recv<W: std::io::Write>(
        w: &mut W,
        rem_pub_key: &[u8; 32],
        rem_url: Option<&str>,
        data: Box<dyn Buf + Send>,
    ) -> std::io::Result<()> {
        if data.remaining() > 512 {
            return Err(crate::Error::id("BootDataOver512B"));
        }
        let b64 = base64::encode(rem_pub_key);
        Tx5PipeWriteRow::new(w)
            .write_field(b"@boot_recv")?
            .write_field(b64.as_bytes())?
            .write_field(rem_url.unwrap_or("").as_bytes())?
            .write_bin(data)?
            .finish()
    }

    /// Encode this instance in the tx5 pipe ipc protocol.
    pub fn encode<W: std::io::Write>(self, w: &mut W) -> std::io::Result<()> {
        match self {
            Self::Tx5PipeHelp { version, info } => {
                Self::tx5_pipe_help(w, &version, &info)
            }
            Self::Error { cmd_id, code, text } => {
                Self::tx5_error(w, &cmd_id, code, &text)
            }
            Self::AppRegOk { cmd_id } => Self::tx5_app_reg_ok(w, &cmd_id),
            Self::SigRegOk { cmd_id, cli_url } => {
                Self::tx5_sig_reg_ok(w, &cmd_id, &cli_url)
            }
            Self::BootRegOk { cmd_id } => Self::tx5_boot_reg_ok(w, &cmd_id),
            Self::SendOk { cmd_id } => Self::tx5_send_ok(w, &cmd_id),
            Self::Recv { rem_url, data } => Self::tx5_recv(w, &rem_url, data),
            Self::BootRecv {
                rem_pub_key,
                rem_url,
                data,
            } => Self::tx5_boot_recv(w, &rem_pub_key, rem_url.as_deref(), data),
        }
    }

    /// Decode a [Tx5PipeResponse] instance from an already parsed
    /// tx5 pipe ipc protocol message. See [asv::AsvParse].
    pub fn decode(mut fields: Vec<Vec<u8>>) -> std::io::Result<Self> {
        if fields.is_empty() {
            return Err(crate::Error::id("EmptyFieldList"));
        }
        match fields.remove(0).as_slice() {
            b"@help" => {
                if fields.len() != 2 {
                    return Err(crate::Error::id("InvalidArgs"));
                }
                let version =
                    String::from_utf8_lossy(&fields.remove(0)).to_string();
                let info =
                    String::from_utf8_lossy(&fields.remove(0)).to_string();
                Ok(Self::Tx5PipeHelp { version, info })
            }
            b"@error" => {
                if fields.len() != 3 {
                    return Err(crate::Error::id("InvalidArgs"));
                }
                let cmd_id =
                    String::from_utf8_lossy(&fields.remove(0)).to_string();
                let code =
                    String::from_utf8_lossy(&fields.remove(0)).to_string();
                let text =
                    String::from_utf8_lossy(&fields.remove(0)).to_string();
                Ok(Self::Error {
                    cmd_id,
                    code: code.parse().map_err(crate::Error::err)?,
                    text,
                })
            }
            b"@app_reg_ok" => {
                if fields.len() != 1 {
                    return Err(crate::Error::id("InvalidArgs"));
                }
                let cmd_id =
                    String::from_utf8_lossy(&fields.remove(0)).to_string();
                Ok(Self::AppRegOk { cmd_id })
            }
            b"@sig_reg_ok" => {
                if fields.len() != 2 {
                    return Err(crate::Error::id("InvalidArgs"));
                }
                let cmd_id =
                    String::from_utf8_lossy(&fields.remove(0)).to_string();
                let cli_url =
                    String::from_utf8_lossy(&fields.remove(0)).to_string();
                Ok(Self::SigRegOk { cmd_id, cli_url })
            }
            b"@boot_reg_ok" => {
                if fields.len() != 1 {
                    return Err(crate::Error::id("InvalidArgs"));
                }
                let cmd_id =
                    String::from_utf8_lossy(&fields.remove(0)).to_string();
                Ok(Self::BootRegOk { cmd_id })
            }
            b"@send_ok" => {
                if fields.len() != 1 {
                    return Err(crate::Error::id("InvalidArgs"));
                }
                let cmd_id =
                    String::from_utf8_lossy(&fields.remove(0)).to_string();
                Ok(Self::SendOk { cmd_id })
            }
            b"@recv" => {
                if fields.len() != 2 {
                    return Err(crate::Error::id("InvalidArgs"));
                }
                let rem_url =
                    String::from_utf8_lossy(&fields.remove(0)).to_string();
                let data = fields.remove(0);
                if data.len() > 16 * 1024 {
                    return Err(crate::Error::id("MsgOver16KiB"));
                }
                let data = Box::new(std::io::Cursor::new(data));
                Ok(Self::Recv { rem_url, data })
            }
            b"@boot_recv" => {
                if fields.len() != 3 {
                    return Err(crate::Error::id("InvalidArgs"));
                }

                let data = fields.remove(0);
                let pk_unsized =
                    base64::decode(data).map_err(crate::Error::err)?;
                if pk_unsized.len() != 32 {
                    return Err(crate::Error::id("InvalidRemPubKeySize"));
                }
                let mut rem_pub_key = [0; 32];
                rem_pub_key.copy_from_slice(&pk_unsized);
                let data = fields.remove(0);
                let rem_url = if data.is_empty() {
                    None
                } else {
                    Some(String::from_utf8_lossy(&data).to_string())
                };
                let data = fields.remove(0);
                if data.len() > 512 {
                    return Err(crate::Error::id("BootDataOver512B"));
                }
                let data = Box::new(std::io::Cursor::new(data));
                Ok(Self::BootRecv {
                    rem_pub_key,
                    rem_url,
                    data,
                })
            }
            oth => Err(crate::Error::err(format!(
                "invalid cmd: {}",
                String::from_utf8_lossy(oth)
            ))),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    const REQ_RES_FIX: &[&[&str]] = &[
        &[
            "app_reg",
            "test1",
            "Ov8rjjg6jzhf7yUlp4S9Q1L9s9wZhaKJGe2mB4pax0k=",
        ],
        &["sig_reg", "test2", "wss://yada"],
        &["boot_reg", "test3", "https://yada", "yada"],
        &["send", "test4", "wss://yada", "yada"],
        &["@help", "test5", "yada\nmultiline"],
        &["@error", "test6", "42", "yada"],
        &["@app_reg_ok", "test7"],
        &["@sig_reg_ok", "test8", "yada"],
        &["@boot_reg_ok", "test9"],
        &["@send_ok", "test10"],
        &["@recv", "test11", "yada"],
        &[
            "@boot_recv",
            "Ov8rjjg6jzhf7yUlp4S9Q1L9s9wZhaKJGe2mB4pax0k=",
            "wss://yada",
            "yada",
        ],
        &[
            "@boot_recv",
            "Ov8rjjg6jzhf7yUlp4S9Q1L9s9wZhaKJGe2mB4pax0k=",
            "wss://yada",
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
            let mut p = asv::AsvParse::default().parse(&by_hand).unwrap();
            assert_eq!(1, p.len());
            let p = p.remove(0);
            let mut enc = Vec::new();
            if is_resp {
                Tx5PipeResponse::decode(p)
                    .unwrap()
                    .encode(&mut enc)
                    .unwrap();
            } else {
                Tx5PipeRequest::decode(p).unwrap().encode(&mut enc).unwrap();
            }
            let mut p = asv::AsvParse::default().parse(&enc).unwrap();
            assert_eq!(1, p.len());
            let p = p.remove(0);
            let mut p = p.iter();
            for field in *field_list {
                let actual = String::from_utf8_lossy(&p.next().unwrap());
                assert_eq!(*field, actual);
            }
        }
    }
}
