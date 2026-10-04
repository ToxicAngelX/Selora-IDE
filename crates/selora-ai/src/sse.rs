//! Hand-rolled SSE parser (PLAN.md §4.7, ARCHITECTURE.md §6.3, RISKS.md §3).
//!
//! SSE is a four-line grammar, so we parse it in-house instead of pulling in
//! an `eventsource` crate. The parser is a pure function over a byte buffer:
//! the caller appends network chunks and calls [`parse_sse_event`] until it
//! returns `Ok(None)`, draining the returned byte count from the front of the
//! buffer after each complete event.

use selora_core::SeloraError;

/// Hard cap on a single event's accumulated `data:` payload (RISKS.md §3).
/// A `data:` field larger than this aborts the stream with
/// `SeloraError::Ai("gateway returned oversized chunk")`.
pub const MAX_DATA_FIELD_BYTES: usize = 1024 * 1024;

/// Slack for the incomplete-line growth guard: a line that has not yet seen
/// its terminator may exceed the cap by the length of the field prefix
/// (`data: ` plus the line ending) before we declare it oversized.
const LINE_GROWTH_SLACK: usize = 1024;

/// UTF-8 byte order mark: stripped once, at the head of the stream.
const UTF8_BOM: &[u8] = &[0xEF, 0xBB, 0xBF];

/// A single parsed server-sent event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    /// The `data:` field values, joined with `\n` per the SSE spec.
    /// `None` when the event carried no `data:` line at all.
    pub data: Option<String>,
    /// The `event:` field value, if the event carried one.
    pub event_type: Option<String>,
}

/// Parse one complete SSE event from the front of `buf`.
///
/// * `Ok(None)` — the buffer does not hold a complete event yet; append more
///   bytes and call again.
/// * `Ok(Some((event, consumed)))` — a complete event; `consumed` bytes (the
///   event block plus its blank-line terminator) must be drained from the
///   front of the buffer.
/// * `Err(SeloraError::Ai(..))` — the event's `data:` payload exceeded
///   [`MAX_DATA_FIELD_BYTES`], or a line grew past the cap without ever
///   being terminated (unbounded-buffer guard).
///
/// Lines starting with `:` are comments and are skipped. `data:` lines are
/// concatenated with `\n`. `\n\n` and `\r\n\r\n` both terminate an event;
/// LF, CRLF and mixed line endings are accepted inside an event block.
pub fn parse_sse_event(buf: &[u8]) -> Result<Option<(SseEvent, usize)>, SeloraError> {
    let bom = if buf.starts_with(UTF8_BOM) {
        UTF8_BOM.len()
    } else {
        0
    };
    let buf = &buf[bom..];

    let mut cursor = 0usize;
    let mut data: Option<Vec<u8>> = None;
    let mut event_type: Option<String> = None;

    loop {
        // Find the next line ending (LF). CR immediately before an LF is
        // part of the line ending, not the line.
        let Some(rel) = buf[cursor..].iter().position(|&b| b == b'\n') else {
            // The line is still arriving. Guard against unbounded growth: a
            // single line may never exceed the data-field cap (plus slack).
            if buf.len() - cursor > MAX_DATA_FIELD_BYTES + LINE_GROWTH_SLACK {
                return Err(oversized_chunk());
            }
            return Ok(None);
        };
        let mut line_end = cursor + rel;
        if line_end > cursor && buf[line_end - 1] == b'\r' {
            line_end -= 1;
        }
        let line = &buf[cursor..line_end];
        let next = cursor + rel + 1; // first byte after the LF

        if line.is_empty() {
            // Blank line: the event terminator (SSE spec dispatch step).
            let data = match data {
                Some(bytes) => Some(String::from_utf8(bytes).map_err(|_| {
                    SeloraError::Ai("gateway sent invalid UTF-8 in SSE data".into())
                })?),
                None => None,
            };
            return Ok(Some((SseEvent { data, event_type }, next + bom)));
        }

        if line.first() != Some(&b':') {
            // Lines starting with `:` are comments: skipped entirely.
            let (name, value) = split_field(line);
            match name {
                b"data" => {
                    let payload = data.get_or_insert_with(Vec::new);
                    let join = usize::from(!payload.is_empty());
                    if payload.len() + join + value.len() > MAX_DATA_FIELD_BYTES {
                        return Err(oversized_chunk());
                    }
                    if join == 1 {
                        payload.push(b'\n');
                    }
                    payload.extend_from_slice(value);
                }
                b"event" => {
                    event_type = std::str::from_utf8(value).ok().map(str::to_owned);
                }
                // `id:`, `retry:` and unknown field names are ignored (SSE spec).
                _ => {}
            }
        }

        cursor = next;
    }
}

/// Split an SSE field line into `(name, value)`, stripping the single
/// leading space the spec allows after the colon. A line with no colon is a
/// field name with an empty value.
fn split_field(line: &[u8]) -> (&[u8], &[u8]) {
    match line.iter().position(|&b| b == b':') {
        Some(pos) => {
            let value = &line[pos + 1..];
            let value = if value.first() == Some(&b' ') {
                &value[1..]
            } else {
                value
            };
            (&line[..pos], value)
        }
        None => (line, &[]),
    }
}

fn oversized_chunk() -> SeloraError {
    SeloraError::Ai("gateway returned oversized chunk".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parse every complete event out of `buf`, draining as we go, and
    /// return them in order.
    fn parse_all(buf: &mut Vec<u8>) -> Result<Vec<SseEvent>, SeloraError> {
        let mut events = Vec::new();
        while let Some((event, consumed)) = parse_sse_event(buf)? {
            assert!(consumed > 0, "parser must always make progress");
            buf.drain(..consumed);
            events.push(event);
        }
        Ok(events)
    }

    fn data_of(event: &SseEvent) -> Option<&str> {
        event.data.as_deref()
    }

    // ---- The 12 mandatory fixtures from RISKS.md §3 ----

    // 1. Normal case.
    #[test]
    fn normal_case() {
        let mut buf = b"data: {\"choices\":[{\"delta\":{}}]}\n\n".to_vec();
        let events = parse_all(&mut buf).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(data_of(&events[0]), Some("{\"choices\":[{\"delta\":{}}]}"));
        assert!(buf.is_empty());
    }

    // 2. Multi-line `data:` fields, concatenated per the SSE spec.
    #[test]
    fn multi_line_data_is_concatenated() {
        let mut buf = b"data: first\ndata: second\n\n".to_vec();
        let events = parse_all(&mut buf).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(data_of(&events[0]), Some("first\nsecond"));
    }

    // 3. Comment lines interspersed with data lines.
    #[test]
    fn comment_lines_are_skipped() {
        let mut buf = b": ping\ndata: x\n: keepalive\ndata: y\n\n".to_vec();
        let events = parse_all(&mut buf).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(data_of(&events[0]), Some("x\ny"));
    }

    // 4. Event with no `data:` line at all: completes, no data.
    #[test]
    fn event_with_no_data_line() {
        let mut buf = b"event: ping\n\n".to_vec();
        let events = parse_all(&mut buf).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(data_of(&events[0]), None);
        assert_eq!(events[0].event_type.as_deref(), Some("ping"));
    }

    // 5. Abrupt close: bytes arrive but the terminator never does.
    #[test]
    fn abrupt_close_returns_none() {
        let mut buf = b"data: {\"x\":1}\n".to_vec();
        assert!(parse_sse_event(&buf).unwrap().is_none());
        // Partial second line, still no terminator.
        buf.extend_from_slice(b"data: {\"par");
        let events = parse_all(&mut buf).unwrap();
        assert!(events.is_empty());
    }

    // 6. Very large single chunk (under the 1 MB cap).
    #[test]
    fn large_single_chunk_under_cap() {
        let payload = "x".repeat(512 * 1024);
        let mut buf = format!("data: {payload}\n\n").into_bytes();
        let events = parse_all(&mut buf).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(data_of(&events[0]), Some(payload.as_str()));
    }

    // 7. A `data:` payload split across two chunk boundaries.
    #[test]
    fn payload_split_across_chunk_boundaries() {
        let mut buf = b"data: {\"par".to_vec();
        assert!(parse_sse_event(&buf).unwrap().is_none(), "incomplete");
        buf.extend_from_slice(b"tial\":true}\n\n");
        let (event, consumed) = parse_sse_event(&buf).unwrap().expect("complete");
        assert_eq!(data_of(&event), Some("{\"partial\":true}"));
        assert_eq!(consumed, buf.len());
    }

    // 7b. A multi-byte UTF-8 character split across chunk boundaries.
    #[test]
    fn utf8_split_across_chunk_boundaries() {
        let mut buf = b"data: \xE2\x82".to_vec(); // first 2 bytes of '€'
        assert!(parse_sse_event(&buf).unwrap().is_none());
        buf.extend_from_slice(b"\xAC\n\n");
        let events = parse_all(&mut buf).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(data_of(&events[0]), Some("\u{20AC}"));
    }

    // 8. CRLF line endings (and a CRLF event terminator).
    #[test]
    fn crlf_line_endings() {
        let mut buf = b"data: hello\r\n\r\n".to_vec();
        let events = parse_all(&mut buf).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(data_of(&events[0]), Some("hello"));
        // Mixed endings: `\n\r\n` is a blank line (LF, then CRLF), so this
        // buffer holds two events, not one.
        let mut buf = b"data: a\n\r\ndata: b\r\n\n".to_vec();
        let events = parse_all(&mut buf).unwrap();
        let datas: Vec<Option<&str>> = events.iter().map(data_of).collect();
        assert_eq!(datas, [Some("a"), Some("b")]);
    }

    // 9. `[DONE]` as the last event.
    #[test]
    fn done_sentinel_as_last_event() {
        let mut buf = b"data: {\"chunk\":1}\n\ndata: [DONE]\n\n".to_vec();
        let events = parse_all(&mut buf).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(data_of(&events[1]), Some("[DONE]"));
    }

    // 10. `[DONE]` followed by trailing data: the parser keeps going; the
    // client is the one that stops at the sentinel.
    #[test]
    fn done_followed_by_trailing_data() {
        let mut buf = b"data: a\n\ndata: [DONE]\n\ndata: b\n\n".to_vec();
        let events = parse_all(&mut buf).unwrap();
        let datas: Vec<Option<&str>> = events.iter().map(data_of).collect();
        assert_eq!(datas, [Some("a"), Some("[DONE]"), Some("b")]);
    }

    // 11. Empty data payload.
    #[test]
    fn empty_data_payload() {
        for input in ["data:\n\n", "data: \n\n"] {
            let mut buf = input.as_bytes().to_vec();
            let events = parse_all(&mut buf).unwrap();
            assert_eq!(events.len(), 1);
            assert_eq!(data_of(&events[0]), Some(""));
        }
    }

    // 12. Oversized data field: the hard cap trips.
    #[test]
    fn oversized_data_field_trips_cap() {
        let payload = "x".repeat(MAX_DATA_FIELD_BYTES + 1);
        let buf = format!("data: {payload}\n\n").into_bytes();
        let err = parse_sse_event(&buf).unwrap_err();
        assert!(matches!(&err, SeloraError::Ai(msg) if msg.contains("oversized")));
    }

    // ---- Beyond the 12: growth and spec edge cases ----

    // A line that grows past the cap without ever being terminated must not
    // grow the buffer without bound.
    #[test]
    fn unterminated_line_past_cap_errors() {
        let mut buf = b"data: ".to_vec();
        buf.extend(std::iter::repeat_n(
            b'x',
            MAX_DATA_FIELD_BYTES + LINE_GROWTH_SLACK + 1,
        ));
        let err = parse_sse_event(&buf).unwrap_err();
        assert!(matches!(&err, SeloraError::Ai(msg) if msg.contains("oversized")));
    }

    // Multiple events in one buffer, plus a trailing partial event.
    #[test]
    fn multiple_events_and_partial_tail() {
        let mut buf = b"data: a\n\ndata: b\n\ndata: c\n".to_vec();
        let events = parse_all(&mut buf).unwrap();
        let datas: Vec<Option<&str>> = events.iter().map(data_of).collect();
        assert_eq!(datas, [Some("a"), Some("b")]);
        assert_eq!(buf, b"data: c\n");
    }

    // Leading blank lines each dispatch a fieldless event (the client skips
    // them): `\n\n` alone is two blank lines.
    #[test]
    fn leading_blank_lines_produce_fieldless_events() {
        let mut buf = b"\n\ndata: x\n\n".to_vec();
        let events = parse_all(&mut buf).unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(data_of(&events[0]), None);
        assert_eq!(data_of(&events[1]), None);
        assert_eq!(data_of(&events[2]), Some("x"));
    }

    // Unknown field names (`id:`, `retry:`) are ignored per the spec.
    #[test]
    fn unknown_fields_are_ignored() {
        let mut buf = b"id: 7\nretry: 100\ndata: x\n\n".to_vec();
        let events = parse_all(&mut buf).unwrap();
        assert_eq!(data_of(&events[0]), Some("x"));
    }

    // A line with no colon is a field name with an empty value: `data` alone
    // contributes an empty data line.
    #[test]
    fn bare_field_name_line() {
        let mut buf = b"data\n\n".to_vec();
        let events = parse_all(&mut buf).unwrap();
        assert_eq!(data_of(&events[0]), Some(""));
    }

    // A UTF-8 BOM at the head of the stream is stripped.
    #[test]
    fn bom_is_stripped() {
        let mut buf = "\u{FEFF}data: x\n\n".as_bytes().to_vec();
        let events = parse_all(&mut buf).unwrap();
        assert_eq!(data_of(&events[0]), Some("x"));
    }

    // Invalid UTF-8 inside a complete event is an error, not mojibake.
    #[test]
    fn invalid_utf8_data_errors() {
        let buf = b"data: \xFF\xFE\n\n".to_vec();
        let err = parse_sse_event(&buf).unwrap_err();
        assert!(matches!(&err, SeloraError::Ai(msg) if msg.contains("UTF-8")));
    }

    // An empty buffer is simply "no complete event yet".
    #[test]
    fn empty_buffer_is_incomplete() {
        assert!(parse_sse_event(b"").unwrap().is_none());
    }
}
