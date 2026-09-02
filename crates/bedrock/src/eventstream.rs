//! The binary framing a streamed Bedrock reply arrives in.
//!
//! Not server-sent events. Bedrock wraps each event in an AWS event-stream frame: a length prelude,
//! a set of typed headers, a payload, and two CRCs. The payload is a JSON object with the event's
//! JSON base64-encoded inside it, which is the reply's actual content.
//!
//! ```text
//! [total_len:u32][headers_len:u32][prelude_crc:u32][headers][payload][message_crc:u32]
//! ```
//!
//! Decoded by hand for the same reason the rest of this crate is: the alternative is the AWS SDK, a
//! very large dependency for one frame format, and the format is a length and two checksums.
//!
//! # Nothing here decides anything
//!
//! This finds where a frame begins and ends, which is transport structure, exactly like the SSE
//! decoder the other backend uses. The bytes inside are handed on with the label they arrived under.
//! The CRCs are checked because a frame that fails one is a frame that was corrupted in transit, and
//! reading a truncated length as a real one would mean waiting forever for bytes that are not coming.

/// The fixed prelude: two lengths and their checksum.
const PRELUDE_BYTES: usize = 12;

/// The trailing checksum over the whole frame.
const MESSAGE_CRC_BYTES: usize = 4;

/// The smallest a frame can be: a prelude, no headers, no payload, and the trailing CRC.
const MIN_FRAME_BYTES: usize = PRELUDE_BYTES + MESSAGE_CRC_BYTES;

/// The largest frame worth believing.
///
/// A length field is four bytes, so a corrupted one can claim four gigabytes. Bounded so a bad
/// length is refused rather than turned into an allocation, and set far above any real event: the
/// largest is a reply's worth of text.
const MAX_FRAME_BYTES: u32 = 16 * 1024 * 1024;

/// Something wrong with the framing itself.
#[derive(Debug, PartialEq, Eq)]
pub enum FrameError {
    /// A length or a checksum did not hold, so where this frame ends is unknown.
    ///
    /// Not recoverable by skipping: the length is how the next frame is found, so a corrupt one
    /// means the position in the stream is lost.
    Corrupt { detail: String },
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Corrupt { detail } => write!(f, "the reply's framing was corrupt: {detail}"),
        }
    }
}

impl std::error::Error for FrameError {}

/// Reassembles frames from bytes that arrive in arbitrarily sized pieces.
///
/// A frame is very often split across reads, and two frames very often arrive in one, so the buffer
/// here is what makes the caller's read size irrelevant.
#[derive(Debug, Default)]
pub struct FrameDecoder {
    buffered: Vec<u8>,
}

impl FrameDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add bytes and take every complete event they finished.
    ///
    /// Returns the JSON payloads, already unwrapped from their frames and base64. An incomplete
    /// frame stays buffered for the next call.
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Vec<u8>>, FrameError> {
        self.buffered.extend_from_slice(bytes);

        let mut events = Vec::new();
        loop {
            match self.take_frame()? {
                Some(payload) => {
                    if let Some(event) = event_from(&payload) {
                        events.push(event);
                    }
                }
                None => return Ok(events),
            }
        }
    }

    /// Whether bytes are held that did not form a whole frame.
    ///
    /// A stream that ended here ended mid-frame, which is a reply that was cut off rather than one
    /// that finished.
    pub fn is_mid_frame(&self) -> bool {
        !self.buffered.is_empty()
    }

    /// Take the next whole frame's payload, if there is one.
    fn take_frame(&mut self) -> Result<Option<Vec<u8>>, FrameError> {
        if self.buffered.len() < PRELUDE_BYTES {
            return Ok(None);
        }

        let total_len = u32_at(&self.buffered, 0);
        let headers_len = u32_at(&self.buffered, 4);
        let prelude_crc = u32_at(&self.buffered, 8);

        if crc32(&self.buffered[..8]) != prelude_crc {
            return Err(FrameError::Corrupt {
                detail: "the length prelude failed its checksum".to_string(),
            });
        }

        // Checked against the prelude's own claim before it is used as a length. Past this the
        // arithmetic below cannot overflow or address outside the frame.
        if total_len > MAX_FRAME_BYTES
            || (total_len as usize) < MIN_FRAME_BYTES
            || headers_len as usize > total_len as usize - MIN_FRAME_BYTES
        {
            return Err(FrameError::Corrupt {
                detail: format!("a frame claimed an impossible size ({total_len} bytes)"),
            });
        }

        let total = total_len as usize;
        if self.buffered.len() < total {
            return Ok(None);
        }

        let frame: Vec<u8> = self.buffered.drain(..total).collect();

        let claimed = u32_at(&frame, total - MESSAGE_CRC_BYTES);
        if crc32(&frame[..total - MESSAGE_CRC_BYTES]) != claimed {
            return Err(FrameError::Corrupt {
                detail: "a frame failed its checksum".to_string(),
            });
        }

        let payload_start = PRELUDE_BYTES + headers_len as usize;
        let payload_end = total - MESSAGE_CRC_BYTES;
        Ok(Some(frame[payload_start..payload_end].to_vec()))
    }
}

/// The event JSON inside a frame's payload.
///
/// The payload is `{"bytes": "<base64 of the event JSON>"}`. A payload that is not that shape is
/// skipped rather than failing the stream: the framing was sound, so the position in the stream is
/// still known, and the API sends frames this does not need.
fn event_from(payload: &[u8]) -> Option<Vec<u8>> {
    let value: serde_json::Value = serde_json::from_slice(payload).ok()?;
    let encoded = value.get("bytes")?.as_str()?;
    decode_base64(encoded)
}

/// Standard base64 with padding, which is what the payload uses.
///
/// Written out rather than pulled from the base64 crate to keep this crate's dependencies to the two
/// it needs; the alphabet is fixed and the decode is a dozen lines.
fn decode_base64(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut accumulated: u32 = 0;
    let mut bits = 0;

    for byte in text.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            b'\n' | b'\r' => continue,
            _ => return None,
        } as u32;

        accumulated = (accumulated << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((accumulated >> bits) as u8);
        }
    }

    Some(out)
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

/// CRC-32, the checksum the framing uses.
///
/// The ordinary reflected polynomial, computed a bit at a time. A table would be faster and this is
/// not the slow end of reading a stream.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in bytes {
        crc ^= *byte as u32;
        for _ in 0..8 {
            let mask = !(crc & 1).wrapping_sub(1);
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a frame the way the service does, so the decoder is tested against the format rather
    /// than against itself.
    fn frame(payload: &[u8]) -> Vec<u8> {
        frame_with_headers(payload, &[])
    }

    fn frame_with_headers(payload: &[u8], headers: &[u8]) -> Vec<u8> {
        let total = (PRELUDE_BYTES + headers.len() + payload.len() + MESSAGE_CRC_BYTES) as u32;
        let mut out = Vec::new();
        out.extend_from_slice(&total.to_be_bytes());
        out.extend_from_slice(&(headers.len() as u32).to_be_bytes());
        out.extend_from_slice(&crc32(&out[..8]).to_be_bytes());
        out.extend_from_slice(headers);
        out.extend_from_slice(payload);
        let crc = crc32(&out);
        out.extend_from_slice(&crc.to_be_bytes());
        out
    }

    /// The payload shape the service uses: the event's JSON, base64'd, inside a wrapper object.
    fn wrapped(event: &str) -> Vec<u8> {
        format!(r#"{{"bytes":"{}"}}"#, to_base64(event.as_bytes())).into_bytes()
    }

    fn to_base64(bytes: &[u8]) -> String {
        const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let b = [
                chunk[0],
                *chunk.get(1).unwrap_or(&0),
                *chunk.get(2).unwrap_or(&0),
            ];
            let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
            out.push(ALPHABET[(n >> 18) as usize & 63] as char);
            out.push(ALPHABET[(n >> 12) as usize & 63] as char);
            out.push(if chunk.len() > 1 {
                ALPHABET[(n >> 6) as usize & 63] as char
            } else {
                '='
            });
            out.push(if chunk.len() > 2 {
                ALPHABET[n as usize & 63] as char
            } else {
                '='
            });
        }
        out
    }

    /// CRC-32 against a known value, so the checksum is the standard one rather than something that
    /// only agrees with itself. "123456789" is the canonical check vector.
    #[test]
    fn the_checksum_is_the_standard_crc32() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    /// Base64 against known vectors, including both padding lengths.
    #[test]
    fn base64_decodes_the_standard_alphabet() {
        assert_eq!(decode_base64("aGVsbG8=").as_deref(), Some(&b"hello"[..]));
        assert_eq!(decode_base64("aGk=").as_deref(), Some(&b"hi"[..]));
        assert_eq!(decode_base64("aGVsbG8h").as_deref(), Some(&b"hello!"[..]));
        assert_eq!(decode_base64("").as_deref(), Some(&b""[..]));
        // The bytes a JSON payload actually carries: '+' and '/' are in the alphabet.
        assert_eq!(decode_base64("+/8=").as_deref(), Some(&[0xFB, 0xFF][..]));
    }

    /// The whole point: an event comes out of a frame with its JSON intact.
    #[test]
    fn an_event_is_recovered_from_a_frame() {
        let mut decoder = FrameDecoder::new();
        let events = decoder
            .push(&frame(&wrapped(r#"{"type":"message_stop"}"#)))
            .expect("decodes");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0], br#"{"type":"message_stop"}"#);
    }

    /// A frame very often arrives split across reads, and a decoder that needed whole frames per
    /// read would lose most of a reply.
    #[test]
    fn a_frame_split_across_reads_is_reassembled() {
        let whole = frame(&wrapped(r#"{"type":"message_stop"}"#));
        let mut decoder = FrameDecoder::new();

        // A byte at a time, which is the worst case and subsumes every other split.
        let mut events = Vec::new();
        for byte in &whole {
            events.extend(decoder.push(&[*byte]).expect("decodes"));
        }
        assert_eq!(events.len(), 1);
        assert_eq!(events[0], br#"{"type":"message_stop"}"#);
        assert!(!decoder.is_mid_frame(), "nothing should be left over");
    }

    /// Several frames very often arrive in one read, and a decoder that took only the first would
    /// drop the rest of the reply.
    #[test]
    fn several_frames_in_one_read_all_come_out() {
        let mut bytes = frame(&wrapped(r#"{"type":"a"}"#));
        bytes.extend(frame(&wrapped(r#"{"type":"b"}"#)));
        bytes.extend(frame(&wrapped(r#"{"type":"c"}"#)));

        let mut decoder = FrameDecoder::new();
        let events = decoder.push(&bytes).expect("decodes");
        assert_eq!(events.len(), 3);
        assert_eq!(events[2], br#"{"type":"c"}"#);
    }

    /// Real frames carry headers naming the event type. The payload starts after them, so a decoder
    /// that ignored the header length would read them as content.
    #[test]
    fn headers_are_skipped_to_find_the_payload() {
        let mut decoder = FrameDecoder::new();
        let events = decoder
            .push(&frame_with_headers(
                &wrapped(r#"{"type":"a"}"#),
                b"\x0b:event-typesome-header-bytes",
            ))
            .expect("decodes");
        assert_eq!(events[0], br#"{"type":"a"}"#);
    }

    /// A stream that ends mid-frame is a reply that was cut off. Without noticing, a truncated reply
    /// is returned as a whole one and the tool call the model was writing simply vanishes.
    #[test]
    fn an_incomplete_frame_is_reported_as_still_mid_frame() {
        let whole = frame(&wrapped(r#"{"type":"message_stop"}"#));
        let mut decoder = FrameDecoder::new();
        let events = decoder.push(&whole[..whole.len() - 3]).expect("decodes");
        assert!(events.is_empty());
        assert!(decoder.is_mid_frame());
    }

    /// A corrupt length is where a decoder hangs or over-allocates: the length is how the next frame
    /// is found, so a bad one has to fail rather than be waited on.
    #[test]
    fn an_impossible_length_is_refused_rather_than_waited_for() {
        let mut prelude = Vec::new();
        prelude.extend_from_slice(&u32::MAX.to_be_bytes());
        prelude.extend_from_slice(&0u32.to_be_bytes());
        prelude.extend_from_slice(&crc32(&prelude[..8]).to_be_bytes());

        let mut decoder = FrameDecoder::new();
        assert!(matches!(
            decoder.push(&prelude),
            Err(FrameError::Corrupt { .. })
        ));
    }

    /// A header length larger than the frame would address past its end.
    #[test]
    fn headers_longer_than_the_frame_are_refused() {
        let mut prelude = Vec::new();
        prelude.extend_from_slice(&64u32.to_be_bytes());
        prelude.extend_from_slice(&1_000u32.to_be_bytes());
        prelude.extend_from_slice(&crc32(&prelude[..8]).to_be_bytes());

        let mut decoder = FrameDecoder::new();
        assert!(matches!(
            decoder.push(&prelude),
            Err(FrameError::Corrupt { .. })
        ));
    }

    /// A frame shorter than its own fixed parts cannot be read, and the subtraction that finds its
    /// payload would underflow.
    #[test]
    fn a_frame_too_short_to_hold_its_own_prelude_is_refused() {
        let mut prelude = Vec::new();
        prelude.extend_from_slice(&4u32.to_be_bytes());
        prelude.extend_from_slice(&0u32.to_be_bytes());
        prelude.extend_from_slice(&crc32(&prelude[..8]).to_be_bytes());

        let mut decoder = FrameDecoder::new();
        assert!(matches!(
            decoder.push(&prelude),
            Err(FrameError::Corrupt { .. })
        ));
    }

    /// The checksums are the only thing distinguishing a corrupted frame from a real one, so a
    /// flipped bit in the body must be caught rather than parsed.
    #[test]
    fn a_frame_that_fails_its_checksum_is_refused() {
        let mut bytes = frame(&wrapped(r#"{"type":"message_stop"}"#));
        let last = bytes.len() - MESSAGE_CRC_BYTES - 1;
        bytes[last] ^= 0xFF;

        let mut decoder = FrameDecoder::new();
        assert!(matches!(
            decoder.push(&bytes),
            Err(FrameError::Corrupt { .. })
        ));
    }

    /// A flipped bit in the prelude is caught by its own checksum, before the length is trusted.
    #[test]
    fn a_corrupt_prelude_is_caught_before_its_length_is_used() {
        let mut bytes = frame(&wrapped(r#"{"type":"a"}"#));
        bytes[1] ^= 0xFF;

        let mut decoder = FrameDecoder::new();
        assert!(matches!(
            decoder.push(&bytes),
            Err(FrameError::Corrupt { .. })
        ));
    }

    /// The framing was sound, so the position in the stream is still known. A payload this does not
    /// recognise is skipped rather than failing a reply that is otherwise arriving fine.
    #[test]
    fn a_payload_that_is_not_a_wrapped_event_is_skipped_not_fatal() {
        let mut bytes = frame(b"not json at all");
        bytes.extend(frame(br#"{"no_bytes_field":true}"#));
        bytes.extend(frame(&wrapped(r#"{"type":"real"}"#)));

        let mut decoder = FrameDecoder::new();
        let events = decoder.push(&bytes).expect("the framing was fine");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0], br#"{"type":"real"}"#);
    }

    /// An empty read is what a quiet connection produces, and it must not be mistaken for an end.
    #[test]
    fn an_empty_read_yields_nothing_and_is_not_an_error() {
        let mut decoder = FrameDecoder::new();
        assert!(decoder.push(&[]).expect("decodes").is_empty());
        assert!(!decoder.is_mid_frame());
    }
}
