//! Types for dealing with the tx5-pipe ipc protocol.

use std::future::Future;

type ArgIterCb<'lt> = Box<dyn FnMut() -> Option<&'lt str> + 'lt>;

/// Iterator helper for Arguments.
pub struct ArgIter<'lt>(ArgIterCb<'lt>);

impl<'lt> ArgIter<'lt> {
    /// Construct a new ArgIter helper.
    pub fn new<Cb: FnMut() -> Option<&'lt str> + 'lt>(cb: Cb) -> Self {
        Self(Box::new(cb))
    }
}

impl<'lt> std::iter::Iterator for ArgIter<'lt> {
    type Item = &'lt str;

    fn next(&mut self) -> Option<Self::Item> {
        self.0()
    }
}

/// A protocol "message".
/// - A string "command".
/// - A list of string "arguments".
/// - A terminating binary item.
pub trait Message {
    /// The message "command".
    fn command(&self) -> &str;

    /// The count of items in the "arguments" list.
    fn argument_count(&self) -> usize;

    /// The list of string "arguments".
    fn arguments(&self) -> ArgIter<'_>;

    /// The terminating binary item.
    fn binary_item(&self) -> &[u8];

    #[cfg(test)]
    fn test_dbg(&self) -> String {
        let mut out = String::new();
        out.push_str(self.command());
        out.push(' ');
        for arg in self.arguments() {
            out.push_str(arg);
            out.push(' ');
        }
        out.push('|');
        out.push_str(
            &String::from_utf8_lossy(self.binary_item())
                .to_string()
                .replace("|", "||"),
        );
        out.push('|');
        out
    }
}

impl Message for (String, Vec<String>, Vec<u8>) {
    fn command(&self) -> &str {
        self.0.as_str()
    }

    fn argument_count(&self) -> usize {
        self.1.len()
    }

    fn arguments(&self) -> ArgIter<'_> {
        let mut iter = self.1.iter();
        ArgIter::new(move || iter.next().map(|s| s.as_str()))
    }

    fn binary_item(&self) -> &[u8] {
        self.2.as_slice()
    }
}

impl<'a, 'b, 'c> Message for (&'a str, &'b [String], &'c [u8]) {
    fn command(&self) -> &str {
        self.0
    }

    fn argument_count(&self) -> usize {
        self.1.len()
    }

    fn arguments(&self) -> ArgIter<'_> {
        let mut iter = self.1.iter();
        ArgIter::new(move || iter.next().map(|s| s.as_str()))
    }

    fn binary_item(&self) -> &[u8] {
        self.2
    }
}

impl<'a, 'b, 'c, 'd> Message for (&'a str, &'b [&'c str], &'d [u8]) {
    fn command(&self) -> &str {
        self.0
    }

    fn argument_count(&self) -> usize {
        self.1.len()
    }

    fn arguments(&self) -> ArgIter<'_> {
        let mut iter = self.1.iter();
        ArgIter::new(move || iter.next().copied())
    }

    fn binary_item(&self) -> &[u8] {
        self.2
    }
}

/// Extension trait for writing [Message]s to tokio::io::AsyncWrite items.
pub trait AsyncMessageWriteExt: tokio::io::AsyncWrite + Unpin {
    /// Write a [Message] to the writer.
    /// Note, this function currently makes a bunch of small writes,
    /// consider wrapping your writer in a BufWriter.
    fn write_message<'a, 'b: 'a>(
        &'a mut self,
        m: &'b dyn Message,
    ) -> std::pin::Pin<Box<dyn Future<Output = std::io::Result<()>> + 'a>> {
        Box::pin(async {
            use tokio::io::AsyncWriteExt;

            self.write_all(m.command().as_bytes()).await?;
            self.write_all(b" ").await?;
            for arg in m.arguments() {
                self.write_all(arg.as_bytes()).await?;
                self.write_all(b" ").await?;
            }

            let b_hdr = format!("b|{}|", m.binary_item().len());
            self.write_all(b_hdr.as_bytes()).await?;
            self.write_all(m.binary_item()).await?;
            self.write_all(b"|\n").await?;

            Ok(())
        })
    }
}

impl<W: tokio::io::AsyncWrite + Unpin + ?Sized> AsyncMessageWriteExt for W {}

#[inline(always)]
#[allow(clippy::match_like_matches_macro)]
fn is_whitespace(b: u8) -> bool {
    match b {
        b' ' | b'\t' | b'\r' | b'\n' => true,
        _ => false,
    }
}

#[inline(always)]
#[allow(clippy::match_like_matches_macro)]
fn is_digit(b: u8) -> bool {
    match b {
        b'0' | b'1' | b'2' | b'3' | b'4' | b'5' | b'6' | b'7' | b'8' | b'9' => {
            true
        }
        _ => false,
    }
}

async fn eat_whitespace<R: tokio::io::AsyncRead + Unpin + ?Sized>(
    r: &mut R,
) -> std::io::Result<u8> {
    use tokio::io::AsyncReadExt;

    loop {
        let b = r.read_u8().await?;
        if !is_whitespace(b) {
            return Ok(b);
        }
    }
}

async fn eat_comment<R: tokio::io::AsyncRead + Unpin + ?Sized>(
    r: &mut R,
) -> std::io::Result<()> {
    use tokio::io::AsyncReadExt;

    loop {
        if r.read_u8().await? == b'\n' {
            return Ok(());
        }
    }
}

async fn read_str_token<R: tokio::io::AsyncRead + Unpin + ?Sized>(
    r: &mut R,
    mut init: Vec<u8>,
) -> std::io::Result<String> {
    use tokio::io::AsyncReadExt;

    loop {
        let b = r.read_u8().await?;
        if is_whitespace(b) {
            return Ok(String::from_utf8_lossy(&init).to_string());
        } else {
            init.push(b);
        }
    }
}

async fn read_txt_token<R: tokio::io::AsyncRead + Unpin + ?Sized>(
    r: &mut R,
) -> std::io::Result<Vec<u8>> {
    use tokio::io::AsyncReadExt;

    let mut out = Vec::new();

    let mut escape = false;

    loop {
        let b = r.read_u8().await?;
        if escape && b == b'|' {
            out.push(b'|');
            escape = false;
        } else if escape {
            if !is_whitespace(b) {
                return Err(crate::Error::id("InvalidCharFollowingTextClose"));
            }
            return Ok(out);
        } else if b == b'|' {
            escape = true;
        } else {
            out.push(b);
        }
    }
}

async fn read_bin_token<R: tokio::io::AsyncRead + Unpin + ?Sized>(
    r: &mut R,
) -> std::io::Result<Vec<u8>> {
    use tokio::io::AsyncReadExt;

    let mut count = Vec::new();

    loop {
        let b = r.read_u8().await?;
        if is_digit(b) {
            count.push(b)
        } else {
            if count.is_empty() {
                return Err(crate::Error::id("ExpectedBinaryLength"));
            }
            if b != b'|' {
                return Err(crate::Error::id("ExpectedBinaryLengthTerminator"));
            }
            break;
        }
    }

    let count: usize = String::from_utf8_lossy(&count).parse().unwrap();

    let mut out = vec![0; count];
    r.read_exact(&mut out[..]).await?;

    if r.read_u8().await? != b'|' {
        return Err(crate::Error::id("ExpectedBinaryTerminator"));
    }

    Ok(out)
}

fn do_return(
    mut args: Vec<String>,
    bin: Vec<u8>,
) -> std::io::Result<(String, Vec<String>, Vec<u8>)> {
    if args.is_empty() {
        return Err(crate::Error::id("InvalidIpcProtocolMessage"));
    }
    let cmd = args.remove(0);
    Ok((cmd, args, bin))
}

type ReadMessageFut<'lt> = Box<
    dyn Future<Output = std::io::Result<(String, Vec<String>, Vec<u8>)>> + 'lt,
>;

/// Extension trait for reading [Message]s from tokio::io::AsyncRead items.
pub trait AsyncMessageReadExt: tokio::io::AsyncRead + Unpin {
    /// Read a [Message] from the reader.
    /// Note, this function currently makes a bunch of small reads,
    /// consider wrapping your reader in a BufReader.
    /// Returns std::io::ErrorKind::UnexpectedEof if the stream ends.
    fn read_message(&mut self) -> std::pin::Pin<ReadMessageFut<'_>> {
        use tokio::io::AsyncReadExt;

        Box::pin(async {
            let mut args = Vec::new();

            loop {
                let mut init = Vec::with_capacity(8);

                let b1 = eat_whitespace(self).await?;

                if b1 == b'#' {
                    eat_comment(self).await?;
                    continue;
                } else if b1 == b'|' {
                    let bin = read_txt_token(self).await?;
                    return do_return(args, bin);
                } else if b1 == b'b' {
                    let b2 = self.read_u8().await?;
                    if is_whitespace(b2) {
                        args.push("b".to_string());
                        continue;
                    } else if b2 == b'|' {
                        let bin = read_bin_token(self).await?;
                        return do_return(args, bin);
                    } else {
                        init.push(b'b');
                        init.push(b2);
                    }
                } else {
                    init.push(b1);
                }

                args.push(read_str_token(self, init).await?);
            }
        })
    }
}

impl<R: tokio::io::AsyncRead + Unpin + ?Sized> AsyncMessageReadExt for R {}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn types() {
        let a1 = (
            "cmd".to_string(),
            vec!["arg".to_string()],
            b"binary".to_vec(),
        )
            .test_dbg();
        let a2 = ("cmd", &["arg".to_string()][..], &b"binary"[..]).test_dbg();
        let a3 = ("cmd", &["arg"][..], &b"binary"[..]).test_dbg();
        assert_eq!("cmd arg |binary|", a1);
        assert_eq!(a1, a2);
        assert_eq!(a1, a3);
    }

    const READ_FIX: &[(&[u8], &[(&str, &[&str], &[u8])])] = &[
        (b"cmd arg |binary|\n", &[("cmd", &["arg"], b"binary")]),
        (b"empty-text ||\n", &[("empty-text", &[], b"")]),
        (b"cmd arg b|3|bin|\n", &[("cmd", &["arg"], b"bin")]),
        (b"cmd b|0||\n", &[("cmd", &[], b"")]),
        (
            br#"# yo
cmd #yo
#yo
  arg # yo oy |aoeu| b|3|aoe|
  |#
  multi||
  line|
"#,
            &[("cmd", &["arg"], b"#\n  multi|\n  line")],
        ),
        (
            br#"
a ||
b || #
c ||||
d || e ||
f b|2||||
"#,
            &[
                ("a", &[], b""),
                ("b", &[], b""),
                ("c", &[], b"|"),
                ("d", &[], b""),
                ("e", &[], b""),
                ("f", &[], b"||"),
            ],
        ),
    ];

    #[tokio::test]
    async fn read() {
        use tokio::io::AsyncWriteExt;

        for (encoded, expected_list) in READ_FIX {
            let (mut send, mut recv) = tokio::io::duplex(7);

            let task = tokio::task::spawn(async move {
                send.write_all(encoded).await.unwrap()
            });

            for expected in *expected_list {
                println!("expecting: {expected:?}");
                let mut actual = recv.read_message().await.unwrap();

                assert_eq!(expected.0, &actual.0);

                for arg in expected.1 {
                    assert_eq!(arg, &actual.1.remove(0));
                }

                if expected.2 != &actual.2 {
                    panic!(
                        "expected: {}, actual: {}",
                        String::from_utf8_lossy(expected.2),
                        String::from_utf8_lossy(&actual.2),
                    );
                }
            }

            assert!(recv.read_message().await.is_err());

            task.await.unwrap();
        }
    }
}
