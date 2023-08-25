//! Types for dealing with the tx5-pipe ipc protocol.

type Message = (String, Vec<String>, Box<dyn bytes::Buf + Send>);

fn check_token(s: &str) -> std::io::Result<()> {
    if s.contains([' ', '\t', '\r', '\n', '|']) {
        Err(crate::Error::id("ArgCannotContainWhitespaceOrPipe"))
    } else {
        Ok(())
    }
}

trait BufExt {
    fn into_string(self) -> String;
    fn into_vec(self) -> Vec<u8>;
}

impl BufExt for Box<dyn bytes::Buf + Send> {
    fn into_string(self) -> String {
        let out = self.into_vec();
        String::from_utf8_lossy(&out).to_string()
    }

    fn into_vec(mut self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.remaining());
        while self.has_remaining() {
            let c = self.chunk();
            out.extend_from_slice(c);
            self.advance(c.len());
        }
        out
    }
}

/// Low-level message encoder for the tx5 pipe ipc protocol.
pub fn tx5_pipe_encode<C, A, I, B>(
    cmd: C,
    args: I,
    mut bin: B,
) -> std::io::Result<Vec<u8>>
where
    C: AsRef<str>,
    A: AsRef<str>,
    I: Iterator<Item = A>,
    B: bytes::Buf,
{
    check_token(cmd.as_ref())?;

    let mut out = Vec::new();
    out.extend_from_slice(cmd.as_ref().as_bytes());
    out.push(b' ');

    for arg in args {
        check_token(arg.as_ref())?;

        out.extend_from_slice(arg.as_ref().as_bytes());
        out.push(b' ');
    }

    let b_hdr = format!("b|{}|", bin.remaining());
    out.extend_from_slice(b_hdr.as_bytes());
    while bin.has_remaining() {
        let c = bin.chunk();
        out.extend_from_slice(c);
        bin.advance(c.len());
    }
    out.push(b'|');
    out.push(b'\n');

    Ok(out)
}

enum DecodeState {
    EatWhitespace,
    EatComment,
    GatherToken(Vec<u8>),
    GatherText(bytes::BytesMut),
    CheckText(bytes::BytesMut),
    CheckBinary,
    BinaryCount(Vec<u8>),
    GatherBinary(usize, bytes::BytesMut),
}

impl std::fmt::Debug for DecodeState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EatWhitespace => f.write_str("EatWhitespace"),
            Self::EatComment => f.write_str("EatComment"),
            Self::GatherToken(t) => {
                write!(f, "GatherToken({:?})", String::from_utf8_lossy(t))
            }
            Self::GatherText(t) => {
                write!(f, "GatherText({:?})", String::from_utf8_lossy(t))
            }
            Self::CheckText(t) => {
                write!(f, "CheckText({:?})", String::from_utf8_lossy(t))
            }
            Self::CheckBinary => f.write_str("CheckBinary"),
            Self::BinaryCount(t) => {
                write!(f, "BinaryCount({:?})", String::from_utf8_lossy(t))
            }
            Self::GatherBinary(c, t) => {
                write!(f, "GatherBinary({c}, {:?})", String::from_utf8_lossy(t))
            }
        }
    }
}

/// Low-level message decoder for the tx5 pipe ipc protocol.
pub struct Tx5PipeDecoder {
    state: Option<DecodeState>,
    working: Vec<String>,
    complete: Vec<Message>,
}

impl Default for Tx5PipeDecoder {
    fn default() -> Self {
        Self {
            state: Some(DecodeState::EatWhitespace),
            working: Vec::new(),
            complete: Vec::new(),
        }
    }
}

impl Tx5PipeDecoder {
    /// Update the decoder with additional incoming bytes,
    /// outputting any complete messages that were parsed.
    pub fn update<E: AsRef<[u8]>>(
        &mut self,
        input: E,
    ) -> std::io::Result<Vec<Message>> {
        let mut input = input.as_ref();

        while !input.is_empty() {
            let state = self.state.take().unwrap();
            input = self.run_state(state, input)?;
        }

        Ok(std::mem::take(&mut self.complete))
    }

    fn run_state<'b>(
        &mut self,
        state: DecodeState,
        input: &'b [u8],
    ) -> std::io::Result<&'b [u8]> {
        // println!("run {state:?} {:?}", String::from_utf8_lossy(input));
        match state {
            DecodeState::EatWhitespace => {
                let (state, input) = self.eat_whitespace(input)?;
                self.state = Some(state);
                Ok(input)
            }
            DecodeState::EatComment => {
                let (state, input) = self.eat_comment(input)?;
                self.state = Some(state);
                Ok(input)
            }
            DecodeState::GatherToken(token) => {
                let (state, input) = self.gather_token(input, token)?;
                self.state = Some(state);
                Ok(input)
            }
            DecodeState::GatherText(text) => {
                let (state, input) = self.gather_text(input, text)?;
                self.state = Some(state);
                Ok(input)
            }
            DecodeState::CheckText(text) => {
                let (state, input) = self.check_text(input, text)?;
                self.state = Some(state);
                Ok(input)
            }
            DecodeState::CheckBinary => {
                let (state, input) = self.check_binary(input)?;
                self.state = Some(state);
                Ok(input)
            }
            DecodeState::BinaryCount(count) => {
                let (state, input) = self.binary_count(input, count)?;
                self.state = Some(state);
                Ok(input)
            }
            DecodeState::GatherBinary(count, bin) => {
                let (state, input) = self.gather_binary(input, count, bin)?;
                self.state = Some(state);
                Ok(input)
            }
        }
    }

    fn do_complete(
        &mut self,
        bin: Box<dyn bytes::Buf + Send>,
    ) -> std::io::Result<()> {
        if self.working.is_empty() {
            return Err(crate::Error::id("MissingCommand"));
        }
        let cmd = self.working.remove(0);
        self.complete
            .push((cmd, std::mem::take(&mut self.working), bin));
        Ok(())
    }

    fn eat_whitespace<'b>(
        &mut self,
        mut input: &'b [u8],
    ) -> std::io::Result<(DecodeState, &'b [u8])> {
        while !input.is_empty() {
            match input[0] {
                b' ' | b'\t' | b'\r' | b'\n' => input = &input[1..],
                b'#' => return Ok((DecodeState::EatComment, &input[1..])),
                b'|' => {
                    return Ok((
                        DecodeState::GatherText(bytes::BytesMut::new()),
                        &input[1..],
                    ))
                }
                b'b' => return Ok((DecodeState::CheckBinary, &input[1..])),
                _ => {
                    return Ok((
                        DecodeState::GatherToken(vec![input[0]]),
                        &input[1..],
                    ))
                }
            }
        }
        Ok((DecodeState::EatWhitespace, input))
    }

    fn eat_comment<'b>(
        &mut self,
        mut input: &'b [u8],
    ) -> std::io::Result<(DecodeState, &'b [u8])> {
        while !input.is_empty() {
            match input[0] {
                b'\n' => return Ok((DecodeState::EatWhitespace, &input[1..])),
                _ => input = &input[1..],
            }
        }
        Ok((DecodeState::EatComment, input))
    }

    fn gather_token<'b>(
        &mut self,
        mut input: &'b [u8],
        mut token: Vec<u8>,
    ) -> std::io::Result<(DecodeState, &'b [u8])> {
        while !input.is_empty() {
            match input[0] {
                b' ' | b'\t' | b'\r' | b'\n' => {
                    self.working
                        .push(String::from_utf8_lossy(&token).to_string());
                    return Ok((DecodeState::EatWhitespace, &input[1..]));
                }
                b'|' => {
                    return Err(crate::Error::id("InvalidToken"));
                }
                _ => {
                    token.push(input[0]);
                    input = &input[1..];
                }
            }
        }
        Ok((DecodeState::GatherToken(token), input))
    }

    fn gather_text<'b>(
        &mut self,
        mut input: &'b [u8],
        mut text: bytes::BytesMut,
    ) -> std::io::Result<(DecodeState, &'b [u8])> {
        while !input.is_empty() {
            match input[0] {
                b'|' => return Ok((DecodeState::CheckText(text), &input[1..])),
                _ => {
                    text.extend_from_slice(&input[0..=0]);
                    input = &input[1..];
                }
            }
        }
        Ok((DecodeState::GatherText(text), input))
    }

    fn check_text<'b>(
        &mut self,
        input: &'b [u8],
        mut text: bytes::BytesMut,
    ) -> std::io::Result<(DecodeState, &'b [u8])> {
        match input[0] {
            b'|' => {
                text.extend_from_slice(&input[0..=0]);
                Ok((DecodeState::GatherText(text), &input[1..]))
            }
            b' ' | b'\t' | b'\r' | b'\n' => {
                self.do_complete(Box::new(text))?;
                Ok((DecodeState::EatWhitespace, &input[1..]))
            }
            _ => Err(crate::Error::id("InvalidTextToken")),
        }
    }

    fn check_binary<'b>(
        &mut self,
        input: &'b [u8],
    ) -> std::io::Result<(DecodeState, &'b [u8])> {
        match input[0] {
            b'|' => Ok((DecodeState::BinaryCount(Vec::new()), &input[1..])),
            b' ' | b'\t' | b'\r' | b'\n' => {
                self.working.push("b".to_string());
                Ok((DecodeState::EatWhitespace, &input[1..]))
            }
            _ => Ok((
                DecodeState::GatherToken(vec![b'b', input[0]]),
                &input[1..],
            )),
        }
    }

    fn binary_count<'b>(
        &mut self,
        mut input: &'b [u8],
        mut count: Vec<u8>,
    ) -> std::io::Result<(DecodeState, &'b [u8])> {
        while !input.is_empty() {
            match input[0] {
                b'0' | b'1' | b'2' | b'3' | b'4' | b'5' | b'6' | b'7'
                | b'8' | b'9' => {
                    count.push(input[0]);
                    input = &input[1..];
                }
                b'|' => {
                    input = &input[1..];
                    let count: usize =
                        String::from_utf8_lossy(&count).parse().unwrap();
                    return Ok((
                        DecodeState::GatherBinary(
                            count,
                            bytes::BytesMut::new(),
                        ),
                        input,
                    ));
                }
                _ => return Err(crate::Error::id("InvalidBinaryToken")),
            }
        }
        Ok((DecodeState::BinaryCount(count), input))
    }

    fn gather_binary<'b>(
        &mut self,
        mut input: &'b [u8],
        mut count: usize,
        mut bin: bytes::BytesMut,
    ) -> std::io::Result<(DecodeState, &'b [u8])> {
        let amt = std::cmp::min(count, input.len());
        if amt > 0 {
            bin.extend_from_slice(&input[..amt]);
            input = &input[amt..];
            count -= amt;
        }

        if input.is_empty() {
            Ok((DecodeState::GatherBinary(count, bin), input))
        } else if input[0] == b'|' {
            self.do_complete(Box::new(bin))?;
            Ok((DecodeState::EatWhitespace, &input[1..]))
        } else {
            Err(crate::Error::id("InvalidPostBinaryToken"))
        }
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
    /// Register an application hash.
    pub fn tx5_app_reg<A>(
        cmd_id: A,
        app_hash: &[u8; 32],
    ) -> std::io::Result<Vec<u8>>
    where
        A: AsRef<str>,
    {
        let b64 = base64::encode(app_hash);
        tx5_pipe_encode("app_reg", [cmd_id].iter(), b64.as_bytes())
    }

    /// Register to be addressable with a signal server.
    pub fn tx5_sig_reg<A, B>(cmd_id: A, sig_url: B) -> std::io::Result<Vec<u8>>
    where
        A: AsRef<str>,
        B: AsRef<str>,
    {
        tx5_pipe_encode("sig_reg", [cmd_id].iter(), sig_url.as_ref().as_bytes())
    }

    /// A request to make this node discoverable on a bootstrap server.
    pub fn tx5_boot_reg<A, B>(
        cmd_id: A,
        boot_url: B,
    ) -> std::io::Result<Vec<u8>>
    where
        A: AsRef<str>,
        B: AsRef<str>,
    {
        tx5_pipe_encode(
            "boot_reg",
            [cmd_id].iter(),
            boot_url.as_ref().as_bytes(),
        )
    }

    /// Send a message to a remote.
    pub fn tx5_send<A, B>(
        cmd_id: A,
        rem_url: A,
        data: B,
    ) -> std::io::Result<Vec<u8>>
    where
        A: AsRef<str>,
        B: bytes::Buf,
    {
        tx5_pipe_encode("send", [cmd_id, rem_url].iter(), data)
    }

    /// Encode this instance in the tx5 pipe ipc protocol.
    pub fn encode(self) -> std::io::Result<Vec<u8>> {
        match self {
            Self::AppReg { cmd_id, app_hash } => {
                Self::tx5_app_reg(cmd_id, &app_hash)
            }
            Self::SigReg { cmd_id, sig_url } => {
                Self::tx5_sig_reg(cmd_id, sig_url)
            }
            Self::BootReg { cmd_id, boot_url } => {
                Self::tx5_boot_reg(cmd_id, boot_url)
            }
            Self::Send {
                cmd_id,
                rem_url,
                data,
            } => Self::tx5_send(cmd_id, rem_url, data),
        }
    }

    /// Decode a Tx5PipeRequest instance from an already parsed
    /// tx5 pipe ipc protocol message. See [Tx5PipeDecoder].
    pub fn decode(
        cmd: String,
        mut args: Vec<String>,
        data: Box<dyn bytes::Buf + Send>,
    ) -> std::io::Result<Self> {
        match cmd.as_str() {
            "app_reg" => {
                if args.len() != 1 {
                    return Err(crate::Error::id("InvalidArgs"));
                }
                let app_hash_unsized = base64::decode(data.into_vec())
                    .map_err(crate::Error::err)?;
                if app_hash_unsized.len() != 32 {
                    return Err(crate::Error::id("InvalidAppHashSize"));
                }
                let mut app_hash = [0; 32];
                app_hash.copy_from_slice(&app_hash_unsized);
                Ok(Self::AppReg {
                    cmd_id: args.remove(0),
                    app_hash,
                })
            }
            "sig_reg" => {
                if args.len() != 1 {
                    return Err(crate::Error::id("InvalidArgs"));
                }
                Ok(Self::SigReg {
                    cmd_id: args.remove(0),
                    sig_url: data.into_string(),
                })
            }
            "boot_reg" => {
                if args.len() != 1 {
                    return Err(crate::Error::id("InvalidArgs"));
                }
                Ok(Self::BootReg {
                    cmd_id: args.remove(0),
                    boot_url: data.into_string(),
                })
            }
            "send" => {
                if args.len() != 2 {
                    return Err(crate::Error::id("InvalidArgs"));
                }
                Ok(Self::Send {
                    cmd_id: args.remove(0),
                    rem_url: args.remove(0),
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
}

impl Tx5PipeResponse {
    /// Send unsolicited help information.
    pub fn tx5_pipe_help<A, B>(version: A, info: B) -> std::io::Result<Vec<u8>>
    where
        A: AsRef<str>,
        B: AsRef<str>,
    {
        tx5_pipe_encode("<help", [version].iter(), info.as_ref().as_bytes())
    }

    /// If you need to send an error response to a command.
    pub fn tx5_error<A, B>(
        cmd_id: A,
        code: u32,
        text: B,
    ) -> std::io::Result<Vec<u8>>
    where
        A: AsRef<str>,
        B: AsRef<str>,
    {
        let code = format!("{code}");
        tx5_pipe_encode(
            "<error",
            [cmd_id.as_ref(), &code].iter(),
            text.as_ref().as_bytes(),
        )
    }

    /// Okay response to a sig_reg request.
    pub fn tx5_sig_reg_ok<A, B>(
        cmd_id: A,
        cli_url: B,
    ) -> std::io::Result<Vec<u8>>
    where
        A: AsRef<str>,
        B: AsRef<str>,
    {
        tx5_pipe_encode(
            "<sig_reg_ok",
            [cmd_id].iter(),
            cli_url.as_ref().as_bytes(),
        )
    }

    /// Okay response to an app_reg request.
    pub fn tx5_app_reg_ok<A>(cmd_id: A) -> std::io::Result<Vec<u8>>
    where
        A: AsRef<str>,
    {
        tx5_pipe_encode(
            "<app_reg_ok",
            [cmd_id].iter(),
            Box::new(bytes::Bytes::new()),
        )
    }

    /// Okay response to a send request.
    pub fn tx5_send_ok<A>(cmd_id: A) -> std::io::Result<Vec<u8>>
    where
        A: AsRef<str>,
    {
        tx5_pipe_encode(
            "<send_ok",
            [cmd_id].iter(),
            Box::new(bytes::Bytes::new()),
        )
    }

    /// Okay response to an boot_reg request.
    pub fn tx5_boot_reg_ok<A>(cmd_id: A) -> std::io::Result<Vec<u8>>
    where
        A: AsRef<str>,
    {
        tx5_pipe_encode(
            "<boot_reg_ok",
            [cmd_id].iter(),
            Box::new(bytes::Bytes::new()),
        )
    }

    /// Receive data from a remote peer.
    pub fn tx5_recv<A, B>(rem_url: A, data: B) -> std::io::Result<Vec<u8>>
    where
        A: AsRef<str>,
        B: bytes::Buf,
    {
        tx5_pipe_encode("<recv", [rem_url].iter(), data)
    }

    /// Encode this instance in the tx5 pipe ipc protocol.
    pub fn encode(self) -> std::io::Result<Vec<u8>> {
        match self {
            Self::Tx5PipeHelp { version, info } => {
                Self::tx5_pipe_help(version, info)
            }
            Self::Error { cmd_id, code, text } => {
                Self::tx5_error(cmd_id, code, text)
            }
            Self::AppRegOk { cmd_id } => Self::tx5_app_reg_ok(cmd_id),
            Self::SigRegOk { cmd_id, cli_url } => {
                Self::tx5_sig_reg_ok(cmd_id, cli_url)
            }
            Self::BootRegOk { cmd_id } => Self::tx5_boot_reg_ok(cmd_id),
            Self::SendOk { cmd_id } => Self::tx5_send_ok(cmd_id),
            Self::Recv { rem_url, data } => Self::tx5_recv(rem_url, data),
        }
    }

    /// Decode a Tx5PipeRequest instance from an already parsed
    /// tx5 pipe ipc protocol message. See [Tx5PipeDecoder].
    pub fn decode(
        cmd: String,
        mut args: Vec<String>,
        data: Box<dyn bytes::Buf + Send>,
    ) -> std::io::Result<Self> {
        match cmd.as_str() {
            "<help" => {
                if args.len() != 1 {
                    return Err(crate::Error::id("InvalidArgs"));
                }
                Ok(Self::Tx5PipeHelp {
                    version: args.remove(0),
                    info: data.into_string(),
                })
            }
            "<error" => {
                if args.len() != 2 {
                    return Err(crate::Error::id("InvalidArgs"));
                }
                Ok(Self::Error {
                    cmd_id: args.remove(0),
                    code: args.remove(0).parse().map_err(crate::Error::err)?,
                    text: data.into_string(),
                })
            }
            "<app_reg_ok" => {
                if args.len() != 1 {
                    return Err(crate::Error::id("InvalidArgs"));
                }
                Ok(Self::AppRegOk {
                    cmd_id: args.remove(0),
                })
            }
            "<sig_reg_ok" => {
                if args.len() != 1 {
                    return Err(crate::Error::id("InvalidArgs"));
                }
                Ok(Self::SigRegOk {
                    cmd_id: args.remove(0),
                    cli_url: data.into_string(),
                })
            }
            "<boot_reg_ok" => {
                if args.len() != 1 {
                    return Err(crate::Error::id("InvalidArgs"));
                }
                Ok(Self::BootRegOk {
                    cmd_id: args.remove(0),
                })
            }
            "<send_ok" => {
                if args.len() != 1 {
                    return Err(crate::Error::id("InvalidArgs"));
                }
                Ok(Self::SendOk {
                    cmd_id: args.remove(0),
                })
            }
            "<recv" => {
                if args.len() != 1 {
                    return Err(crate::Error::id("InvalidArgs"));
                }
                Ok(Self::Recv {
                    rem_url: args.remove(0),
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

    const READ_FIX: &[(&[u8], &[(&str, &[&str], &[u8])])] = &[
        (
            b"test1 arg |bin\x00ry|\n",
            &[("test1", &["arg"], b"bin\x00ry")],
        ),
        (b"test2 ||\n", &[("test2", &[], b"")]),
        (
            b"test3 arg b|3|b\x00n|\n",
            &[("test3", &["arg"], b"b\x00n")],
        ),
        (b"test4 b|0||\n", &[("test4", &[], b"")]),
        (
            br#"# yo
test5 #yo
#yo
  arg # yo oy |aoeu| b|3|aoe|
  |#
  multi||
  line|
"#,
            &[("test5", &["arg"], b"#\n  multi|\n  line")],
        ),
        (
            br#"
test6 ||
test7 || #
test8 ||||
test9 || test10 ||
test11 b|2||||
test12 b barg b|0||
"#,
            &[
                ("test6", &[], b""),
                ("test7", &[], b""),
                ("test8", &[], b"|"),
                ("test9", &[], b""),
                ("test10", &[], b""),
                ("test11", &[], b"||"),
                ("test12", &["b", "barg"], b""),
            ],
        ),
    ];

    fn check_read_fix(
        exp_list: &[(&str, &[&str], &[u8])],
        mut res: Vec<Message>,
    ) {
        for exp in exp_list {
            if res.is_empty() {
                panic!("expected {exp:?}, actual was missing");
            }
            let res = res.remove(0);

            let mut ok = true;

            if exp.0 != &res.0 {
                ok = false;
            }

            if exp.1.len() != res.1.len() {
                ok = false;
            } else {
                for (idx, exp) in exp.1.iter().enumerate() {
                    if exp != &res.1[idx] {
                        ok = false;
                        break;
                    }
                }
            }

            let exp_data = String::from_utf8_lossy(exp.2).to_string();
            let res_data = res.2.into_string();

            if exp_data != res_data {
                ok = false;
            }

            if !ok {
                panic!(
                    "expected: {exp:?}, actual: [({:?}, {:?}, {:?}]",
                    res.0, res.1, res_data,
                );
            }
        }
    }

    #[test]
    fn test_decoder_block() {
        for (enc, exp_list) in READ_FIX {
            let mut d = Tx5PipeDecoder::default();
            let res = d.update(enc).unwrap();
            check_read_fix(exp_list, res);
        }
    }

    #[test]
    fn test_decoder_bytes() {
        for (enc, exp_list) in READ_FIX {
            let mut d = Tx5PipeDecoder::default();
            let mut res = Vec::new();
            for b in enc.iter() {
                res.append(&mut d.update(&[*b][..]).unwrap());
            }
            check_read_fix(exp_list, res);
        }
    }

    const REQ_RES_FIX: &[(&str, &[&str], &[u8])] = &[
        (
            "app_reg",
            &["test1"],
            b"Ov8rjjg6jzhf7yUlp4S9Q1L9s9wZhaKJGe2mB4pax0k=",
        ),
        ("sig_reg", &["test2"], b"wss://yada"),
        ("boot_reg", &["test3"], b"https://yada"),
        ("send", &["test4", "wss://yada"], b"yada"),
        ("<help", &["test5"], b"yada\nmultiline"),
        ("<error", &["test6", "42"], b"yada"),
        ("<app_reg_ok", &["test7"], b""),
        ("<sig_reg_ok", &["test8"], b"yada"),
        ("<boot_reg_ok", &["test9"], b""),
        ("<send_ok", &["test10"], b""),
        ("<recv", &["test11"], b"yada"),
    ];

    fn cmp_res(a: Tx5PipeResponse, b: Tx5PipeResponse) {
        let a = a.encode().unwrap();
        let b = b.encode().unwrap();
        let a = String::from_utf8_lossy(&a);
        let b = String::from_utf8_lossy(&b);
        assert_eq!(a, b);
    }

    fn cmp_req(a: Tx5PipeRequest, b: Tx5PipeRequest) {
        let a = a.encode().unwrap();
        let b = b.encode().unwrap();
        let a = String::from_utf8_lossy(&a);
        let b = String::from_utf8_lossy(&b);
        assert_eq!(a, b);
    }

    #[test]
    fn request_response() {
        let mut decoder = Tx5PipeDecoder::default();

        for (cmd, args, data) in REQ_RES_FIX {
            let cmd = cmd.to_string();
            let args: Vec<_> = args.iter().map(|s| s.to_string()).collect();
            let data = data.to_vec();
            println!("({cmd:?}, {args:?}, {})", String::from_utf8_lossy(&data));
            if cmd.starts_with("<") {
                let exp1 = Tx5PipeResponse::decode(
                    cmd.clone(),
                    args.clone(),
                    Box::new(std::io::Cursor::new(data.clone())),
                )
                .unwrap();
                let exp2 = Tx5PipeResponse::decode(
                    cmd,
                    args,
                    Box::new(std::io::Cursor::new(data)),
                )
                .unwrap();
                let encoded = exp1.encode().unwrap();
                let mut actual = decoder.update(encoded).unwrap();
                assert_eq!(1, actual.len());
                let (cmd, args, data) = actual.remove(0);
                let actual = Tx5PipeResponse::decode(cmd, args, data).unwrap();
                cmp_res(exp2, actual);
            } else {
                let exp1 = Tx5PipeRequest::decode(
                    cmd.clone(),
                    args.clone(),
                    Box::new(std::io::Cursor::new(data.clone())),
                )
                .unwrap();
                let exp2 = Tx5PipeRequest::decode(
                    cmd,
                    args,
                    Box::new(std::io::Cursor::new(data)),
                )
                .unwrap();
                let encoded = exp1.encode().unwrap();
                let mut actual = decoder.update(encoded).unwrap();
                assert_eq!(1, actual.len());
                let (cmd, args, data) = actual.remove(0);
                let actual = Tx5PipeRequest::decode(cmd, args, data).unwrap();
                cmp_req(exp2, actual);
            }
        }
    }
}
