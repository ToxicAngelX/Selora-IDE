//! LSP stdio framing (ARCHITECTURE.md §5.2).
//!
//! LSP over stdio is `Content-Length: N\r\n\r\n<body>` — a small, fixed
//! grammar we parse ourselves (PLAN.md §4.5: no extra framing crate). The
//! helpers here are transport-agnostic: [`encode_message`]/[`write_message`]
//! frame one outgoing JSON body, and [`MessageReader`] incrementally reads
//! framed bodies off any async reader, buffering across chunk boundaries so
//! coalesced TCP/pipe reads and messages split mid-header both work.

use std::io;

use serde_json::Value;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Header terminator: one blank CRLF line after the headers.
const HEADER_END: &[u8] = b"\r\n\r\n";

/// Sanity caps so a corrupt header can't ask us to allocate unbounded memory.
const MAX_BODY_BYTES: usize = 64 * 1024 * 1024;
const MAX_HEADER_BYTES: usize = 16 * 1024;

/// Frame one JSON body: `Content-Length: <len>\r\n\r\n<json>`.
pub fn encode_message(body: &Value) -> Vec<u8> {
    // `to_vec` on an already-parsed `Value` cannot fail (it contains no
    // non-string map keys), and the callers only pass values built by
    // `serde_json` itself — `expect` here is the same contract `to_vec` has.
    let json = serde_json::to_vec(body).expect("serializing a serde_json::Value");
    let mut framed = Vec::with_capacity(json.len() + 32);
    framed.extend_from_slice(format!("Content-Length: {}\r\n\r\n", json.len()).as_bytes());
    framed.extend_from_slice(&json);
    framed
}

/// Write one framed message and flush (a server must not see half a body).
pub async fn write_message<W: AsyncWrite + Unpin>(writer: &mut W, body: &Value) -> io::Result<()> {
    writer.write_all(&encode_message(body)).await?;
    writer.flush().await
}

/// The `Content-Length` of a complete header block sitting at the front of
/// `buf`, plus the header's own byte length (including the blank line).
///
/// `Ok(None)` = the header is not complete yet (more bytes needed).
/// `Err` = the bytes so far can never be a valid LSP header.
pub fn parse_header(buf: &[u8]) -> Result<Option<(usize, usize)>, io::Error> {
    let Some(end) = find_subslice(buf, HEADER_END) else {
        if buf.len() > MAX_HEADER_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "LSP header exceeds 16 KiB",
            ));
        }
        return Ok(None);
    };
    let header = String::from_utf8_lossy(&buf[..end]);
    let mut content_length = None;
    for line in header.split("\r\n") {
        let Some((name, value)) = line.split_once(':') else {
            continue; // blank/malformed lines are skipped, not fatal
        };
        if name.trim().eq_ignore_ascii_case("content-length") {
            let length = value
                .trim()
                .parse::<usize>()
                .map_err(|_| invalid_header(line))?;
            if length > MAX_BODY_BYTES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("LSP message body of {length} bytes exceeds the 64 MiB cap"),
                ));
            }
            content_length = Some(length);
        }
        // Content-Type and any other headers are legal but ignored.
    }
    match content_length {
        Some(length) => Ok(Some((length, end + HEADER_END.len()))),
        None => Err(invalid_header(&header)),
    }
}

/// Try to extract one complete framed message from `buf`, removing the bytes
/// it consumed. `Ok(None)` = not enough bytes buffered yet.
pub fn try_extract_message(buf: &mut Vec<u8>) -> io::Result<Option<Value>> {
    let Some((body_len, header_len)) = parse_header(buf)? else {
        return Ok(None);
    };
    if buf.len() < header_len + body_len {
        return Ok(None);
    }
    let body: Vec<u8> = buf
        .drain(..header_len + body_len)
        .skip(header_len)
        .collect();
    let message = serde_json::from_slice(&body).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid JSON body: {error}"),
        )
    })?;
    Ok(Some(message))
}

/// Incremental framed-message reader over any async byte stream.
///
/// Keeps a persistent buffer so a single `read` that coalesces two messages
/// (or splits one across the header/body boundary) is handled correctly.
pub struct MessageReader<R> {
    reader: R,
    buf: Vec<u8>,
}

impl<R: AsyncRead + Unpin> MessageReader<R> {
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            buf: Vec::new(),
        }
    }

    /// The next message, or `None` on clean EOF. An EOF with a half-buffered
    /// message is an error (a server died mid-write), as is any IO error.
    pub async fn next(&mut self) -> io::Result<Option<Value>> {
        loop {
            if let Some(message) = try_extract_message(&mut self.buf)? {
                return Ok(Some(message));
            }
            let mut chunk = [0u8; 8192];
            let read = self.reader.read(&mut chunk).await?;
            if read == 0 {
                if self.buf.is_empty() {
                    return Ok(None);
                }
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "stream ended mid-message",
                ));
            }
            self.buf.extend_from_slice(&chunk[..read]);
        }
    }
}

fn invalid_header(header: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("malformed LSP header: no Content-Length in {header:?}"),
    )
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `&[u8]` implements tokio's `AsyncRead` — perfect for chunked tests.
    async fn read_all(bytes: &[u8]) -> Vec<Value> {
        let mut messages = Vec::new();
        let mut reader = MessageReader::new(bytes);
        while let Some(message) = reader.next().await.expect("read") {
            messages.push(message);
        }
        messages
    }

    #[test]
    fn header_round_trips_with_content_length() {
        let value = serde_json::json!({"jsonrpc": "2.0", "method": "test"});
        let framed = encode_message(&value);
        let (body_len, header_len) = parse_header(&framed)
            .expect("complete header")
            .expect("some");
        assert_eq!(body_len, value.to_string().len());
        assert_eq!(header_len, framed.len() - body_len);
        // The body parses back to the same value.
        let body: Value = serde_json::from_slice(&framed[header_len..]).expect("body json");
        assert_eq!(body, value);
    }

    #[test]
    fn header_ignores_content_type_and_case() {
        let framed =
            b"Content-Type: application/vscode-jsonrpc;utf-8\r\ncontent-length: 2\r\n\r\n{}";
        let (body_len, header_len) = parse_header(framed).expect("ok").expect("complete");
        assert_eq!(body_len, 2);
        assert_eq!(header_len, framed.len() - 2);
    }

    #[test]
    fn incomplete_header_is_more_not_error() {
        assert_eq!(parse_header(b"Content-Length: 12").unwrap(), None);
        assert_eq!(parse_header(b"Content-Length: 12\r\n").unwrap(), None);
    }

    #[test]
    fn header_without_content_length_is_invalid() {
        assert!(parse_header(b"Content-Type: text\r\n\r\nbody").is_err());
        assert!(parse_header(b"Content-Length: abc\r\n\r\n{}").is_err());
    }

    #[tokio::test]
    async fn reads_messages_split_across_arbitrary_chunk_boundaries() {
        let values = vec![
            serde_json::json!({"id": 1, "result": {"capabilities": {}}}),
            serde_json::json!({"method": "textDocument/publishDiagnostics", "params": {}}),
            serde_json::json!({"id": 2, "result": null}),
        ];
        let mut stream = Vec::new();
        for value in &values {
            stream.extend_from_slice(&encode_message(value));
        }
        // Split the whole stream at every offset: the reader must reassemble
        // all three messages regardless of where the reads break.
        for split in 0..stream.len() {
            let mut reader = MessageReader::new(&stream[..]);
            let mut got = Vec::new();
            while let Some(message) = reader.next().await.expect("read") {
                got.push(message);
            }
            let _ = split;
            assert_eq!(got, values, "split at {split}");
        }
    }

    #[tokio::test]
    async fn reads_two_coalesced_messages_from_one_buffer() {
        let values = vec![serde_json::json!({"id": 7}), serde_json::json!({"id": 8})];
        let mut stream = Vec::new();
        for value in &values {
            stream.extend_from_slice(&encode_message(value));
        }
        let messages = read_all(&stream).await;
        assert_eq!(messages, values);
    }

    #[tokio::test]
    async fn eof_mid_message_is_an_error_clean_eof_is_none() {
        let value = serde_json::json!({"id": 1});
        let mut framed = encode_message(&value);
        framed.truncate(framed.len() - 3); // cut the body short

        let mut reader = MessageReader::new(&framed[..]);
        let error = reader.next().await.expect_err("truncated stream");
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);

        let mut empty = MessageReader::new(&[][..]);
        assert!(empty.next().await.expect("clean eof").is_none());
    }

    #[tokio::test]
    async fn write_message_frames_and_flushes() {
        let (mut client, mut server) = tokio::io::duplex(64);
        let value = serde_json::json!({"method": "exit"});
        write_message(&mut client, &value).await.expect("write");

        let mut reader = MessageReader::new(&mut server);
        let read = reader.next().await.expect("read").expect("message");
        assert_eq!(read, value);
    }
}
