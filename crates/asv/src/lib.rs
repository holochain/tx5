#![deny(missing_docs)]
#![deny(warnings)]
#![deny(unsafe_code)]
//! A separated value encoder and parser.
//!
//! # Example
//!
//! ```
//! # use asv::*;
//! const FIELD: &str = "smiley -\n🙃";
//!
//! let mut enc = AsvEncoder::default();
//!
//! enc.field("test");
//! enc.field(r#"multi
//! line"#);
//! enc.field(FIELD);
//! enc.binary_bytes(FIELD);
//! enc.finish_row();
//!
//! let mut expected = String::new();
//! expected.push_str("test");
//! expected.push(' ');
//! expected.push_str("\"multi\nline\"");
//! expected.push(' ');
//! expected.push_str(&format!("`13|{FIELD}`"));
//! expected.push(' ');
//! expected.push_str(&format!("`13|{FIELD}`"));
//! expected.push('\n');
//!
//! let bytes = enc.drain().into_vec();
//!
//! assert_eq!(expected, String::from_utf8_lossy(&bytes));
//!
//! let mut parser = AsvParser::default();
//! let mut result =  parser.parse(bytes).unwrap();
//!
//! assert_eq!(1, result.len());
//!
//! for mut row in result {
//!     assert_eq!(4, row.len());
//!     assert_eq!("test", &row.remove(0).into_string().unwrap());
//!     assert_eq!("multi\nline", &row.remove(0).into_string().unwrap());
//!     assert_eq!(FIELD, &row.remove(0).into_string().unwrap());
//!     assert_eq!(FIELD, &row.remove(0).into_string().unwrap());
//! }
//! ```

use bytes::*;
use std::collections::VecDeque;
use std::io::Result;

fn err_other<E>(error: E) -> std::io::Error
where
    E: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    std::io::Error::new(std::io::ErrorKind::Other, error.into())
}

macro_rules! delim {
    () => {
        b',' | b' ' | b'\t' | b'\r'
    };
}

macro_rules! digit {
    () => {
        b'0' | b'1' | b'2' | b'3' | b'4' | b'5' | b'6' | b'7' | b'8' | b'9'
    };
}

macro_rules! url {
    () => {
        b'a' | b'b' | b'c' | b'd' | b'e' | b'f' | b'g' | b'h' |
        b'i' | b'j' | b'k' | b'l' | b'm' | b'n' | b'o' | b'p' |
        b'q' | b'r' | b's' | b't' | b'u' | b'v' | b'w' | b'x' |
        b'y' | b'z' | b'A' | b'B' | b'C' | b'D' | b'E' | b'F' |
        b'G' | b'H' | b'I' | b'J' | b'K' | b'L' | b'M' | b'N' |
        b'O' | b'P' | b'Q' | b'R' | b'S' | b'T' | b'U' | b'V' |
        b'W' | b'X' | b'Y' | b'Z' | b'0' | b'1' | b'2' | b'3' |
        b'4' | b'5' | b'6' | b'7' | b'8' | b'9' | b'-' | b'.' |
        b'_' | b'~' | b':' | b'/' | b'?' | b'#' | b'[' | b']' |
        b'@' | b'!' | b'$' | b'&' | b'\'' | b'(' | b')' | b'*' |
        b'+' | b';' | b'=' | b'%'
    };
}

/// A boxed buffer.
pub type BoxedBuf = Box<dyn Buf + 'static + Send>;

enum AsvBufferUnit {
    Bytes(Bytes),
    Boxed(BoxedBuf),
}

impl AsvBufferUnit {
    fn from_bytes<B: Into<Bytes>>(b: B) -> Self {
        Self::Bytes(b.into())
    }

    fn from_boxed(b: BoxedBuf) -> Self {
        Self::Boxed(b)
    }

    fn extend_bytes_mut(mut self, m: &mut BytesMut) {
        while self.has_remaining() {
            let c = self.chunk();
            m.extend_from_slice(c);
            self.advance(c.len());
        }
    }
}

impl Buf for AsvBufferUnit {
    fn advance(&mut self, cnt: usize) {
        match self {
            Self::Bytes(b) => b.advance(cnt),
            Self::Boxed(b) => b.advance(cnt),
        }
    }

    fn chunk(&self) -> &[u8] {
        match self {
            Self::Bytes(b) => b.chunk(),
            Self::Boxed(b) => b.chunk(),
        }
    }

    fn remaining(&self) -> usize {
        match self {
            Self::Bytes(b) => b.remaining(),
            Self::Boxed(b) => b.remaining(),
        }
    }
}

/// A separated value working buffer.
#[derive(Default)]
pub struct AsvBuffer(VecDeque<AsvBufferUnit>);

impl Buf for AsvBuffer {
    fn advance(&mut self, mut cnt: usize) {
        while cnt > 0 && !self.0.is_empty() {
            let amt = std::cmp::min(self.0[0].remaining(), cnt);
            self.0[0].advance(amt);
            cnt -= amt;
            if !self.0[0].has_remaining() {
                self.0.pop_front();
            }
        }
    }

    fn chunk(&self) -> &[u8] {
        if self.0.is_empty() {
            &[]
        } else {
            self.0[0].chunk()
        }
    }

    fn remaining(&self) -> usize {
        let mut out = 0;
        for i in self.0.iter() {
            out += i.remaining()
        }
        out
    }
}

impl AsvBuffer {
    /// Read this buffer into a `Vec<u8>`.
    pub fn into_vec(mut self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.remaining());
        while self.has_remaining() {
            let c = self.chunk();
            out.extend_from_slice(c);
            self.advance(c.len());
        }
        out
    }

    /// Read this buffer into a `String`.
    pub fn into_string(self) -> Result<String> {
        String::from_utf8(self.into_vec()).map_err(err_other)
    }

    fn push_back(&mut self, b: AsvBufferUnit) {
        self.0.push_back(b);
    }

    /// Append an item onto this buffer.
    pub fn push_back_bytes<B: Into<Bytes>>(&mut self, b: B) {
        self.push_back(AsvBufferUnit::from_bytes(b));
    }

    /// Append an item onto this buffer.
    pub fn push_back_boxed(&mut self, b: BoxedBuf) {
        self.push_back(AsvBufferUnit::from_boxed(b));
    }
}

/// A separated value encoder.
pub struct AsvEncoder {
    buf: AsvBuffer,
    work: BytesMut,
    newline: bool,
}

impl Default for AsvEncoder {
    fn default() -> Self {
        Self {
            buf: AsvBuffer::default(),
            work: BytesMut::default(),
            newline: true,
        }
    }
}

impl AsvEncoder {
    /// Extract the data currently encoded.
    /// Don't forget to finish your row.
    pub fn drain(&mut self) -> AsvBuffer {
        self.check_work();
        std::mem::take(&mut self.buf)
    }

    /// Write the row delimiter.
    pub fn finish_row(&mut self) {
        if !self.newline {
            self.newline = true;
            self.work.extend_from_slice(b"\n");
        }
    }

    /// Encode an explicitly binary formatted field.
    /// Don't forget to finish your row.
    pub fn binary_bytes<B: Into<Bytes>>(&mut self, b: B) {
        self.binary_priv(AsvBufferUnit::from_bytes(b))
    }

    /// Encode an explicitly binary formatted field.
    /// Don't forget to finish your row.
    pub fn binary_boxed(&mut self, b: BoxedBuf) {
        self.binary_priv(AsvBufferUnit::from_boxed(b))
    }

    fn binary_priv(&mut self, b: AsvBufferUnit) {
        let mut size = b.remaining();
        let size_str = format!("{size}").into_bytes();
        size += size_str.len();

        // opening tick + len delimiter
        // (but NOT the closing tick, that goes in the next work)
        size += 2;

        if self.newline {
            self.newline = false;
            self.work.reserve(size);
            self.work.extend_from_slice(b"`"); // open
        } else {
            size += 1;
            self.work.reserve(size);
            self.work.extend_from_slice(b" `"); // field delim + open
        }

        self.work.extend_from_slice(&size_str);
        self.work.extend_from_slice(b"|"); // len delim
        self.check_push(b);
        self.work.extend_from_slice(b"`"); // close
    }

    /// Encode a field using heuristics to determine format.
    /// Don't forget to finish your row.
    pub fn field<B: Into<Bytes>>(&mut self, b: B) {
        let b = b.into();

        // special case for empty field, must be quoted
        // (or binary, but that takes up more space)
        if !b.has_remaining() {
            if self.newline {
                self.newline = false;
                self.work.extend_from_slice(b"\"\"");
            } else {
                self.work.extend_from_slice(b" \"\"");
            }
            return;
        }

        // use binary for anything > 64 bytes as base64
        if b.remaining() > 86 {
            self.binary_priv(AsvBufferUnit::Bytes(b));
            return;
        }

        let mut should_quote = false;

        for c in b.iter() {
            match c {
                url!() => (),
                delim!() | b'\n' => should_quote = true,
                _ => {
                    self.binary_priv(AsvBufferUnit::Bytes(b));
                    return;
                }
            }
        }

        if should_quote {
            if self.newline {
                self.newline = false;
                self.work.extend_from_slice(b"\"");
            } else {
                self.work.extend_from_slice(b" \"");
            }
            self.check_push(AsvBufferUnit::Bytes(b));
            self.work.extend_from_slice(b"\"");
        } else {
            if self.newline {
                self.newline = false;
            } else {
                self.work.extend_from_slice(b" ");
            }
            self.check_push(AsvBufferUnit::Bytes(b));
        }
    }

    // -- private -- //

    #[inline(always)]
    fn check_work(&mut self) {
        if !self.work.is_empty() {
            self.buf
                .push_back_bytes(std::mem::take(&mut self.work).freeze());
        }
    }

    fn check_push(&mut self, b: AsvBufferUnit) {
        // somewhat arbitrary limit for balancing memory copying vs thrashing
        // (note this is less than the magic number 1024 to leave space
        // for the closing delim, field delim, and opening bytes of
        // quote or binary format fields)
        if self.work.remaining() + b.remaining() > 1015 {
            self.check_work();
            // an additionally arbitrary number. how do we decide it's
            // better to have the closing delim be written on its own,
            // or included in the data?
            if b.remaining() > 256 {
                self.buf.push_back(b);
            } else {
                b.extend_bytes_mut(&mut self.work);
            }
        } else {
            b.extend_bytes_mut(&mut self.work);
        }
    }
}

enum State {
    EatDelim,
    ParseField { working: AsvBuffer, is_quoted: bool },
    ParseBinHdr { count: Vec<u8> },
    ParseBin { count: usize, working: AsvBuffer },
}

/*
impl std::fmt::Debug for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EatDelim => f.write_str("EatDelim"),
            Self::ParseField { working, is_quoted } => f
                .debug_struct("ParseField")
                .field("working", &String::from_utf8_lossy(&working.to_vec()))
                .field("is_quoted", &is_quoted)
                .finish(),
            Self::ParseBinHdr { count } => f
                .debug_struct("ParseBinHdr")
                .field("count", &String::from_utf8_lossy(count))
                .finish(),
            Self::ParseBin { count, working } => f
                .debug_struct("ParseBinHdr")
                .field("count", count)
                .field("working", &String::from_utf8_lossy(&working.to_vec()))
                .finish(),
        }
    }
}
*/

/// A separated value parser.
pub struct AsvParser {
    state: Option<State>,
    working: Vec<AsvBuffer>,
    complete: Vec<Vec<AsvBuffer>>,
}

impl Default for AsvParser {
    fn default() -> Self {
        Self {
            state: Some(State::EatDelim),
            working: Vec::new(),
            complete: Vec::new(),
        }
    }
}

impl AsvParser {
    /// Parse a chunk of incoming data. This is a stream parser, it is valid
    /// to provide chunked data to this parser.
    pub fn parse<B: Into<Bytes>>(
        &mut self,
        input: B,
    ) -> Result<Vec<Vec<AsvBuffer>>> {
        let mut input = input.into();

        while input.has_remaining() {
            let state = self.state.take().unwrap();
            input = self.run_state(state, input)?;
        }

        Ok(std::mem::take(&mut self.complete))
    }

    fn run_state(&mut self, state: State, input: Bytes) -> Result<Bytes> {
        // println!("run {state:?} {:?}", String::from_utf8_lossy(&input));
        match state {
            State::EatDelim => {
                let (state, input) = self.eat_delim(input)?;
                self.state = Some(state);
                Ok(input)
            }
            State::ParseField { working, is_quoted } => {
                let (state, input) =
                    self.parse_field(working, is_quoted, input)?;
                self.state = Some(state);
                Ok(input)
            }
            State::ParseBinHdr { count } => {
                let (state, input) = self.parse_bin_hdr(count, input)?;
                self.state = Some(state);
                Ok(input)
            }
            State::ParseBin { count, working } => {
                let (state, input) = self.parse_bin(count, working, input)?;
                self.state = Some(state);
                Ok(input)
            }
        }
    }

    fn eat_delim(&mut self, mut input: Bytes) -> Result<(State, Bytes)> {
        while input.has_remaining() {
            match input[0] {
                b'\n' => {
                    if !self.working.is_empty() {
                        self.complete.push(std::mem::take(&mut self.working));
                    }
                    input = input.slice(1..);
                }
                delim!() => input = input.slice(1..),
                b'`' => {
                    return Ok((
                        State::ParseBinHdr { count: Vec::new() },
                        input.slice(1..),
                    ));
                }
                b'"' => {
                    return Ok((
                        State::ParseField {
                            working: AsvBuffer::default(),
                            is_quoted: true,
                        },
                        input.slice(1..),
                    ));
                }
                url!() => {
                    let working = AsvBuffer::default();
                    return Ok((
                        State::ParseField {
                            working,
                            is_quoted: false,
                        },
                        input,
                    ));
                }
                oth => {
                    return Err(err_other(format!(
                        "InvalidDelimByte: 0x{oth:02X}"
                    )))
                }
            }
        }
        Ok((State::EatDelim, input))
    }

    fn parse_field(
        &mut self,
        mut working: AsvBuffer,
        is_quoted: bool,
        mut input: Bytes,
    ) -> Result<(State, Bytes)> {
        let mut idx = 0;

        while idx < input.remaining() {
            match input[idx] {
                b'\n' => {
                    if !is_quoted {
                        if idx > 0 {
                            working.push_back_bytes(input.slice(..idx));
                        }
                        input = input.slice(idx + 1..);
                        self.working.push(working);
                        self.complete.push(std::mem::take(&mut self.working));
                        return Ok((State::EatDelim, input));
                    }
                }
                b'"' => {
                    if is_quoted {
                        if idx > 0 {
                            working.push_back_bytes(input.slice(..idx));
                        }
                        input = input.slice(idx + 1..);
                        self.working.push(working);
                        return Ok((State::EatDelim, input));
                    } else {
                        return Err(err_other("InvalidQuoteInRawField"));
                    }
                }
                delim!() => {
                    if !is_quoted {
                        if idx > 0 {
                            working.push_back_bytes(input.slice(..idx));
                        }
                        input = input.slice(idx + 1..);
                        self.working.push(working);
                        return Ok((State::EatDelim, input));
                    }
                }
                url!() => (),
                oth => {
                    return Err(err_other(format!(
                        "InvalidFieldByte: 0x{oth:02X}"
                    )))
                }
            }
            idx += 1;
        }

        if input.has_remaining() {
            working.push_back_bytes(input);
        }

        Ok((State::ParseField { working, is_quoted }, Bytes::new()))
    }

    fn parse_bin_hdr(
        &mut self,
        mut count: Vec<u8>,
        mut input: Bytes,
    ) -> Result<(State, Bytes)> {
        while input.has_remaining() {
            match input[0] {
                b'|' => {
                    let count: usize =
                        String::from_utf8_lossy(&count).parse().unwrap();
                    return Ok((
                        State::ParseBin {
                            count,
                            working: AsvBuffer::default(),
                        },
                        input.slice(1..),
                    ));
                }
                digit!() => {
                    count.push(input[0]);
                    input = input.slice(1..);
                }
                oth => {
                    return Err(err_other(format!(
                        "InvalidBinHdrByte: 0x{oth:02X}"
                    )))
                }
            }
        }
        Ok((State::ParseBinHdr { count }, input))
    }

    fn parse_bin(
        &mut self,
        mut count: usize,
        mut working: AsvBuffer,
        mut input: Bytes,
    ) -> Result<(State, Bytes)> {
        if count > 0 {
            let amt = std::cmp::min(count, input.len());
            working.push_back_bytes(input.slice(..amt));
            count -= amt;
            input = input.slice(amt..);
        }

        if count > 0 || input.is_empty() {
            return Ok((State::ParseBin { count, working }, input));
        }

        if input[0] == b'`' {
            self.working.push(working);
            Ok((State::EatDelim, input.slice(1..)))
        } else {
            Err(err_other(format!(
                "InvalidBinFooterByte: 0x{:02X}",
                input[0]
            )))
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    // Fixtures for items that convert into utf8 bytes
    const FIX_UTF8: &[(&[u8], &[&[&str]])] = &[
        (
            b"smiley%20-%0D%0A%F0%9F%99%83 \"smiley -\r\n%F0%9F%99%83\" `14|smiley -\r\n\xf0\x9f\x99\x83` \n",
            &[
                &[
                    "smiley%20-%0D%0A%F0%9F%99%83",
                    "smiley -\r\n%F0%9F%99%83",
                    "smiley -\r\n🙃",
                ],
            ],
        ),
        (
            b"empty \"\" field\n", &[&["empty", "", "field"]]
        ),
    ];

    fn check_fix_utf8(expected: &[&[&str]], actual: Vec<Vec<AsvBuffer>>) {
        assert_eq!(expected.len(), actual.len());
        let mut actual = actual.into_iter();
        for expected_line in expected.iter() {
            let actual_line = actual.next().unwrap();

            assert_eq!(expected_line.len(), actual_line.len());

            let mut actual_line = actual_line.into_iter();
            for expected_field in expected_line.iter() {
                let actual_field = actual_line.next().unwrap();
                assert_eq!(
                    expected_field,
                    &String::from_utf8_lossy(&actual_field.into_vec()),
                );
            }
        }
    }

    #[test]
    fn test_fixtures_utf8_whole() {
        for (enc, expected) in FIX_UTF8 {
            let mut p = AsvParser::default();
            let result = p.parse(Bytes::from_static(enc)).unwrap();
            check_fix_utf8(expected, result);
        }
    }

    #[test]
    fn test_fixtures_utf8_by_byte() {
        for (enc, expected) in FIX_UTF8 {
            let mut p = AsvParser::default();
            let mut result = Vec::new();

            for b in enc.iter() {
                result.append(
                    &mut p.parse(Bytes::copy_from_slice(&[*b])).unwrap(),
                );
            }

            check_fix_utf8(expected, result);
        }
    }
}
