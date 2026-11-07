//! The LSP base protocol: messages framed by a `Content-Length` header.

use std::io::{self, BufRead, Read, Write};

/// Largest message body accepted (larger ones are skipped).
pub const MAX_MESSAGE_BYTES: usize = 64 * 1024 * 1024;
/// Longest header line.
const MAX_HEADER_LINE: usize = 8 * 1024;
/// Most header lines of one message.
const MAX_HEADERS: usize = 32;

/// One message read from the client.
#[derive(Debug, PartialEq, Eq)]
pub enum Frame {
    /// The message body (JSON, not yet parsed).
    Body(Vec<u8>),
    /// A body larger than the limit, read and dropped.
    TooLarge(usize),
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// Reads the next message; `None` at the end of the input. A header without
/// `Content-Length`, an unreadable length or input ending inside a message
/// is an error: the stream cannot be followed after it.
pub fn read_message(r: &mut impl BufRead) -> io::Result<Option<Frame>> {
    read_limited(r, MAX_MESSAGE_BYTES)
}

fn read_limited(r: &mut impl BufRead, limit: usize) -> io::Result<Option<Frame>> {
    let mut length: Option<usize> = None;
    let mut headers = 0;
    loop {
        let mut line = Vec::new();
        let n = r
            .by_ref()
            .take(MAX_HEADER_LINE as u64 + 1)
            .read_until(b'\n', &mut line)?;
        if n == 0 {
            if headers == 0 {
                return Ok(None);
            }
            return Err(invalid("the input ended inside a message header"));
        }
        if line.len() > MAX_HEADER_LINE {
            return Err(invalid("a message header line is too long"));
        }
        let text = std::str::from_utf8(&line)
            .map_err(|_| invalid("a message header is not ASCII"))?
            .trim_end_matches(['\r', '\n']);
        if text.is_empty() {
            match length {
                Some(_) => break,
                // Blank lines between messages.
                None if headers == 0 => continue,
                None => return Err(invalid("a message header has no Content-Length")),
            }
        }
        headers += 1;
        if headers > MAX_HEADERS {
            return Err(invalid("a message has too many header lines"));
        }
        let Some((name, value)) = text.split_once(':') else {
            return Err(invalid(format!("not a message header: {text:?}")));
        };
        if name.trim().eq_ignore_ascii_case("content-length") {
            let n = value
                .trim()
                .parse::<usize>()
                .map_err(|_| invalid(format!("bad Content-Length {:?}", value.trim())))?;
            length = Some(n);
        }
    }
    let length = length.unwrap_or(0);
    if length > limit {
        let skipped = io::copy(&mut r.by_ref().take(length as u64), &mut io::sink())?;
        if skipped < length as u64 {
            return Err(invalid("the input ended inside a message"));
        }
        return Ok(Some(Frame::TooLarge(length)));
    }
    let mut body = vec![0; length];
    r.read_exact(&mut body)
        .map_err(|_| invalid("the input ended inside a message"))?;
    Ok(Some(Frame::Body(body)))
}

/// Writes one message and flushes it.
pub fn write_message(w: &mut impl Write, message: &serde_json::Value) -> io::Result<()> {
    let body = serde_json::to_vec(message).map_err(io::Error::other)?;
    write!(w, "Content-Length: {}\r\n\r\n", body.len())?;
    w.write_all(&body)?;
    w.flush()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn read_all(input: &[u8], limit: usize) -> (Vec<Frame>, Option<String>) {
        let mut r = input;
        let mut frames = Vec::new();
        loop {
            match read_limited(&mut r, limit) {
                Ok(Some(f)) => frames.push(f),
                Ok(None) => return (frames, None),
                Err(e) => return (frames, Some(e.to_string())),
            }
        }
    }

    #[test]
    fn reads_framed_messages_with_extra_headers() {
        let input = b"Content-Length: 2\r\n\r\n{}content-length:4\r\nContent-Type: application/vscode-jsonrpc; charset=utf-8\r\n\r\nnull\r\n";
        let (frames, err) = read_all(input, MAX_MESSAGE_BYTES);
        assert_eq!(
            frames,
            vec![Frame::Body(b"{}".to_vec()), Frame::Body(b"null".to_vec())]
        );
        assert_eq!(err, None);
    }

    #[test]
    fn rejects_broken_headers_and_truncated_bodies() {
        for (input, message) in [
            (&b"Content-Type: x\r\n\r\n{}"[..], "no Content-Length"),
            (b"Content-Length: two\r\n\r\n", "bad Content-Length"),
            (b"Content-Length: 10\r\n\r\n{}", "ended inside a message"),
            (b"Content-Length: 2\r\n", "ended inside a message header"),
            (b"garbage\r\n\r\n", "not a message header"),
            (b"Content-Length: 99\r\n\r\n{}", "ended inside a message"),
        ] {
            let (_, err) = read_all(input, 16);
            assert!(err.as_deref().unwrap().contains(message), "{err:?}");
        }
        let long = format!("X-Long: {}\r\n", "a".repeat(MAX_HEADER_LINE));
        let (_, err) = read_all(long.as_bytes(), 16);
        assert!(err.unwrap().contains("too long"));
    }

    #[test]
    fn skips_oversized_bodies() {
        let input =
            b"Content-Length: 20\r\n\r\n{\"a\": \"01234567890\"}Content-Length: 2\r\n\r\n{}";
        let (frames, err) = read_all(input, 16);
        assert_eq!(
            frames,
            vec![Frame::TooLarge(20), Frame::Body(b"{}".to_vec())]
        );
        assert_eq!(err, None);
    }

    #[test]
    fn writes_the_length_in_bytes() {
        let mut out = Vec::new();
        write_message(&mut out, &serde_json::json!({"a": "\u{e9}"})).unwrap();
        assert_eq!(
            out,
            "Content-Length: 10\r\n\r\n{\"a\":\"\u{e9}\"}".as_bytes()
        );
    }
}
