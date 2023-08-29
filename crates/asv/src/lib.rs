#![deny(missing_docs)]
#![deny(warnings)]
#![deny(unsafe_code)]
//! A separated value encoder and parser.
//!
//! # Example
//!
//! ```
//! # use asv::*;
//! # use asv::asv_encoder::*;
//! const FIELD: &str = "smiley -\n🙃";
//!
//! let mut enc = Vec::new();
//!
//! // write a field in the "raw" format:
//! enc.extend_from_slice(&write_raw_field(FIELD.as_bytes()));
//!
//! // field delimiter
//! enc.extend_from_slice(FIELD_DELIM);
//!
//! // write a field in the "quoted" format:
//! enc.extend_from_slice(&write_quote_field(FIELD.as_bytes()));
//!
//! // field delimiter
//! enc.extend_from_slice(FIELD_DELIM);
//!
//! // write a field in the "binary" format:
//! enc.extend_from_slice(&write_bin_header(FIELD.as_bytes().len()));
//! enc.extend_from_slice(FIELD.as_bytes());
//! enc.extend_from_slice(BIN_FOOTER);
//!
//! // row delimiter
//! enc.extend_from_slice(ROW_DELIM);
//!
//! let mut expected = String::new();
//! expected.push_str("smiley%20-%0A%F0%9F%99%83");
//! expected.push(' ');
//! expected.push_str(r#""smiley -
//! %F0%9F%99%83""#);
//! expected.push(' ');
//! expected.push_str(r#"`13|smiley -
//! 🙃`"#);
//! expected.push('\n');
//!
//! assert_eq!(expected, String::from_utf8_lossy(&enc));
//!
//! let mut parser = AsvParse::default();
//! let result = parser.parse(&enc).unwrap();
//! assert_eq!(1, result.len());
//! for row in result {
//!     assert_eq!(3, row.len());
//!     for field in row {
//!         assert_eq!("smiley -\n🙃", String::from_utf8_lossy(&field));
//!     }
//! }
//! ```

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

macro_rules! hex_alpha {
    () => {
        b'a' | b'b'
            | b'c'
            | b'd'
            | b'e'
            | b'f'
            | b'A'
            | b'B'
            | b'C'
            | b'D'
            | b'E'
            | b'F'
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
        b'+' | b';' | b'='
    };
}

/// A separated value encoder.
pub mod asv_encoder {
    /// Delimiter between rows.
    pub const ROW_DELIM: &[u8] = b"\n";

    /// Delimiter between fields.
    pub const FIELD_DELIM: &[u8] = b" ";

    /// Binary field footer.
    pub const BIN_FOOTER: &[u8] = b"`";

    /// What is the best output format for given data.
    pub enum BestFmt {
        /// Data should be encoded using [write_bin_header],
        /// writing the binary data, and including the [BIN_FOOTER].
        Bin,

        /// Data should be encoded using [write_raw_field].
        Raw,

        /// Data should be encoded using [write_quote_field].
        Quote,
    }

    /// Determine the best output format for given data.
    pub fn get_best_fmt(data: &[u8]) -> BestFmt {
        // Anything bigger than a 64 byte hash encoded in base64.
        if data.len() > 86 {
            return BestFmt::Bin;
        }

        let mut should_quote = false;

        for b in data.iter() {
            match b {
                url!() => (),
                delim!() | b'\n' => should_quote = true,
                _ => return BestFmt::Bin,
            }
        }

        if should_quote {
            BestFmt::Quote
        } else {
            BestFmt::Raw
        }
    }

    /// Write a header to a binary formatted field.
    /// Don't forget to write your actual binary data,
    /// followed by the binary format footer.
    /// This is the most efficient way to encode data
    /// for machine readability.
    pub fn write_bin_header(data_len: usize) -> Vec<u8> {
        format!("`{}|", data_len).into_bytes()
    }

    /// Write a raw field in asv format.
    /// This will result in a lot of percent encoding,
    /// and data copying, so is not very efficient for
    /// large amounts of data.
    pub fn write_raw_field(data: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(data.len());
        for b in data {
            match b {
                url!() => out.push(*b),
                _ => out.append(&mut format!("%{:02X}", b).into_bytes()),
            }
        }
        out
    }

    /// Write a quoted field in asv format.
    /// This will result in a lot of percent encoding,
    /// and data copying, so is not very efficient for
    /// large amounts of data.
    pub fn write_quote_field(data: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(data.len() + 2);
        out.push(b'"');
        for b in data {
            match b {
                url!() | delim!() | b'\n' => out.push(*b),
                _ => out.append(&mut format!("%{:02X}", b).into_bytes()),
            }
        }
        out.push(b'"');
        out
    }
}

enum State {
    EatDelim,
    ParseField {
        working: Vec<u8>,
        is_quoted: bool,
    },
    ParseFieldPct {
        working: Vec<u8>,
        is_quoted: bool,
        pct: Vec<u8>,
    },
    ParseBinHdr {
        count: Vec<u8>,
    },
    ParseBin {
        count: usize,
        working: Vec<u8>,
    },
}

impl std::fmt::Debug for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EatDelim => f.write_str("EatDelim"),
            Self::ParseField { working, is_quoted } => f
                .debug_struct("ParseField")
                .field("working", &String::from_utf8_lossy(working))
                .field("is_quoted", &is_quoted)
                .finish(),
            Self::ParseFieldPct {
                working,
                is_quoted,
                pct,
            } => f
                .debug_struct("ParseFieldPct")
                .field("working", &String::from_utf8_lossy(working))
                .field("is_quoted", is_quoted)
                .field("pct", &String::from_utf8_lossy(pct))
                .finish(),
            Self::ParseBinHdr { count } => f
                .debug_struct("ParseBinHdr")
                .field("count", &String::from_utf8_lossy(count))
                .finish(),
            Self::ParseBin { count, working } => f
                .debug_struct("ParseBinHdr")
                .field("count", count)
                .field("working", &String::from_utf8_lossy(working))
                .finish(),
        }
    }
}

/// A separated value parser.
pub struct AsvParse {
    state: Option<State>,
    working: Vec<Vec<u8>>,
    complete: Vec<Vec<Vec<u8>>>,
}

impl Default for AsvParse {
    fn default() -> Self {
        Self {
            state: Some(State::EatDelim),
            working: Vec::new(),
            complete: Vec::new(),
        }
    }
}

impl AsvParse {
    /// Parse a chunk of incoming data. This is a stream parser, it is valid
    /// to provide chunked data to this parser.
    pub fn parse(&mut self, mut input: &[u8]) -> Result<Vec<Vec<Vec<u8>>>> {
        while !input.is_empty() {
            let state = self.state.take().unwrap();
            input = self.run_state(state, input)?;
        }

        Ok(std::mem::take(&mut self.complete))
    }

    fn run_state<'b>(
        &mut self,
        state: State,
        input: &'b [u8],
    ) -> Result<&'b [u8]> {
        // println!("run {state:?} {:?}", String::from_utf8_lossy(input));
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
            State::ParseFieldPct {
                working,
                is_quoted,
                pct,
            } => {
                let (state, input) =
                    self.parse_field_pct(working, is_quoted, pct, input)?;
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

    fn eat_delim<'b>(
        &mut self,
        mut input: &'b [u8],
    ) -> Result<(State, &'b [u8])> {
        while !input.is_empty() {
            match input[0] {
                b'\n' => {
                    if !self.working.is_empty() {
                        self.complete.push(std::mem::take(&mut self.working));
                    }
                    input = &input[1..];
                }
                delim!() => input = &input[1..],
                b'`' => {
                    return Ok((
                        State::ParseBinHdr { count: Vec::new() },
                        &input[1..],
                    ));
                }
                b'"' => {
                    return Ok((
                        State::ParseField {
                            working: Vec::new(),
                            is_quoted: true,
                        },
                        &input[1..],
                    ));
                }
                url!() => {
                    let mut working = Vec::new();
                    working.extend_from_slice(&[input[0]][..]);
                    return Ok((
                        State::ParseField {
                            working,
                            is_quoted: false,
                        },
                        &input[1..],
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

    fn parse_field<'b>(
        &mut self,
        mut working: Vec<u8>,
        is_quoted: bool,
        mut input: &'b [u8],
    ) -> Result<(State, &'b [u8])> {
        while !input.is_empty() {
            match input[0] {
                url!() => {
                    working.extend_from_slice(&[input[0]][..]);
                    input = &input[1..];
                }
                delim!() => {
                    if is_quoted {
                        working.extend_from_slice(&[input[0]][..]);
                        input = &input[1..];
                    } else {
                        self.working.push(working);
                        return Ok((State::EatDelim, &input[1..]));
                    }
                }
                b'\n' => {
                    if is_quoted {
                        working.extend_from_slice(&[input[0]][..]);
                        input = &input[1..];
                    } else {
                        self.working.push(working);
                        self.complete.push(std::mem::take(&mut self.working));
                        return Ok((State::EatDelim, &input[1..]));
                    }
                }
                b'"' => {
                    if is_quoted {
                        self.working.push(working);
                        return Ok((State::EatDelim, &input[1..]));
                    } else {
                        return Err(err_other("InvalidQuoteInRawField"));
                    }
                }
                b'%' => {
                    return Ok((
                        State::ParseFieldPct {
                            working,
                            is_quoted,
                            pct: Vec::new(),
                        },
                        &input[1..],
                    ))
                }
                oth => {
                    return Err(err_other(format!(
                        "InvalidFieldByte: 0x{oth:02X}"
                    )))
                }
            }
        }
        Ok((State::ParseField { working, is_quoted }, input))
    }

    fn parse_field_pct<'b>(
        &mut self,
        mut working: Vec<u8>,
        is_quoted: bool,
        mut pct: Vec<u8>,
        mut input: &'b [u8],
    ) -> Result<(State, &'b [u8])> {
        while !input.is_empty() {
            match input[0] {
                digit!() | hex_alpha!() => {
                    pct.push(input[0]);
                    input = &input[1..];
                    if pct.len() == 2 {
                        let byte = u8::from_str_radix(
                            &String::from_utf8_lossy(&pct),
                            16,
                        )
                        .unwrap();
                        working.extend_from_slice(&[byte][..]);
                        return Ok((
                            State::ParseField { working, is_quoted },
                            input,
                        ));
                    }
                }
                oth => {
                    return Err(err_other(format!(
                        "InvalidPctByte: 0x{oth:02X}"
                    )))
                }
            }
        }
        Ok((
            State::ParseFieldPct {
                working,
                is_quoted,
                pct,
            },
            input,
        ))
    }

    fn parse_bin_hdr<'b>(
        &mut self,
        mut count: Vec<u8>,
        mut input: &'b [u8],
    ) -> Result<(State, &'b [u8])> {
        while !input.is_empty() {
            match input[0] {
                b'|' => {
                    let count: usize =
                        String::from_utf8_lossy(&count).parse().unwrap();
                    return Ok((
                        State::ParseBin {
                            count,
                            working: Vec::with_capacity(count),
                        },
                        &input[1..],
                    ));
                }
                digit!() => {
                    count.push(input[0]);
                    input = &input[1..];
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

    fn parse_bin<'b>(
        &mut self,
        mut count: usize,
        mut working: Vec<u8>,
        mut input: &'b [u8],
    ) -> Result<(State, &'b [u8])> {
        if count > 0 {
            let amt = std::cmp::min(count, input.len());
            working.extend_from_slice(&input[..amt]);
            count -= amt;
            input = &input[amt..];
        }

        if count > 0 || input.is_empty() {
            return Ok((State::ParseBin { count, working }, input));
        }

        if input[0] == b'`' {
            self.working.push(working);
            Ok((State::EatDelim, &input[1..]))
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
                    "smiley -\r\n🙃",
                    "smiley -\r\n🙃",
                    "smiley -\r\n🙃",
                ],
            ],
        )
    ];

    fn check_fix_utf8(expected: &[&[&str]], actual: Vec<Vec<Vec<u8>>>) {
        let mut actual = actual.iter();
        for expected_line in expected.iter() {
            let actual_line = actual.next().unwrap();
            let mut actual_line = actual_line.iter();
            for expected_field in expected_line.iter() {
                let actual_field = actual_line.next().unwrap();
                assert_eq!(
                    expected_field,
                    &String::from_utf8_lossy(&actual_field),
                );
            }
        }
    }

    #[test]
    fn test_fixtures_utf8_whole() {
        for (enc, expected) in FIX_UTF8 {
            let mut p = AsvParse::default();
            let result = p.parse(enc).unwrap();
            check_fix_utf8(expected, result);
        }
    }

    #[test]
    fn test_fixtures_utf8_by_byte() {
        for (enc, expected) in FIX_UTF8 {
            let mut p = AsvParse::default();
            let mut result = Vec::new();

            for b in enc.iter() {
                result.append(&mut p.parse(&[*b]).unwrap());
            }

            check_fix_utf8(expected, result);
        }
    }
}
