//! The binary framing a streamed Bedrock reply arrives in.
//!
//! Not server-sent events. Bedrock wraps each event in an AWS event-stream frame: a length prelude,
//! a set of typed headers, a payload, and two CRCs. The payload is the event's JSON, and which event
//! it is is a header rather than a field of the body.
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
//! This finds where a frame begins and ends and reads the name the framing gave it, which is
//! transport structure, exactly like the SSE decoder the other backend uses. The bytes inside are
//! handed on with the label they arrived under. The CRCs are checked because a frame that fails one
//! is a frame that was corrupted in transit, and reading a truncated length as a real one would mean
//! waiting forever for bytes that are not coming.

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

/// The header naming which event a frame carries.
const EVENT_TYPE_HEADER: &[u8] = b":event-type";

/// The header value type for a string, which is what an event name is.
const STRING_VALUE: u8 = 7;

/// One event out of the stream: the name its frame gave it, and the JSON it carried.
#[derive(Debug, PartialEq, Eq)]
pub struct Event {
    pub name: String,
    pub payload: Vec<u8>,
}

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

/// One whole frame, in the two pieces that matter: the headers, which name the event, and the
/// payload, which is it.
struct Frame {
    headers: Vec<u8>,
    payload: Vec<u8>,
}

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
    /// An incomplete frame stays buffered for the next call.
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Event>, FrameError> {
        self.buffered.extend_from_slice(bytes);

        let mut events = Vec::new();
        loop {
            match self.take_frame()? {
                // A frame whose headers name no event is skipped rather than failing the stream:
                // the framing was sound, so the position in the stream is still known, and the API
                // sends frames this does not need.
                Some(frame) => {
                    if let Some(name) = event_name(&frame.headers) {
                        events.push(Event {
                            name,
                            payload: frame.payload,
                        });
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

    /// Take the next whole frame, if there is one.
    fn take_frame(&mut self) -> Result<Option<Frame>, FrameError> {
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
        Ok(Some(Frame {
            headers: frame[PRELUDE_BYTES..payload_start].to_vec(),
            payload: frame[payload_start..payload_end].to_vec(),
        }))
    }
}

/// The name a frame's headers give its event, if they give one.
///
/// A header is a name, a value type, and a value whose width the type decides:
///
/// ```text
/// [name_len:u8][name][value_type:u8][value]
/// ```
///
/// Every header is walked rather than only the first, because the one naming the event is not
/// always at the front. A value of a type this does not know has a width this cannot skip, so the
/// walk stops there rather than reading whatever follows as a header name.
fn event_name(headers: &[u8]) -> Option<String> {
    let mut at = 0usize;

    while at < headers.len() {
        let name_len = *headers.get(at)? as usize;
        at += 1;
        let name = headers.get(at..at.checked_add(name_len)?)?;
        at += name_len;

        let value_type = *headers.get(at)?;
        at += 1;

        let width = match value_type {
            // A boolean's value is its own type, so it occupies no bytes.
            0 | 1 => 0,
            2 => 1,
            3 => 2,
            4 => 4,
            // A long and a timestamp are both eight bytes.
            5 | 8 => 8,
            // A byte array and a string are both preceded by their length.
            6 | STRING_VALUE => {
                let stated = headers.get(at..at.checked_add(2)?)?;
                at += 2;
                u16::from_be_bytes([stated[0], stated[1]]) as usize
            }
            9 => 16,
            _ => return None,
        };

        let value = headers.get(at..at.checked_add(width)?)?;
        at += width;

        if value_type == STRING_VALUE && name == EVENT_TYPE_HEADER {
            return String::from_utf8(value.to_vec()).ok();
        }
    }

    None
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
    fn frame(name: &str, payload: &[u8]) -> Vec<u8> {
        frame_with_headers(payload, &string_header(EVENT_TYPE_HEADER, name.as_bytes()))
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

    fn string_header(name: &[u8], value: &[u8]) -> Vec<u8> {
        let mut out = vec![name.len() as u8];
        out.extend_from_slice(name);
        out.push(STRING_VALUE);
        out.extend_from_slice(&(value.len() as u16).to_be_bytes());
        out.extend_from_slice(value);
        out
    }

    /// CRC-32 against a known value, so the checksum is the standard one rather than something that
    /// only agrees with itself. "123456789" is the canonical check vector.
    #[test]
    fn the_checksum_is_the_standard_crc32() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    /// The whole point: an event comes out of a frame under its name, with its JSON intact.
    #[test]
    fn an_event_is_recovered_from_a_frame() {
        let mut decoder = FrameDecoder::new();
        let events = decoder
            .push(&frame("messageStop", br#"{"stopReason":"end_turn"}"#))
            .expect("decodes");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].name, "messageStop");
        assert_eq!(events[0].payload, br#"{"stopReason":"end_turn"}"#);
    }

    /// A frame very often arrives split across reads, and a decoder that needed whole frames per
    /// read would lose most of a reply.
    #[test]
    fn a_frame_split_across_reads_is_reassembled() {
        let whole = frame("messageStop", br#"{"stopReason":"end_turn"}"#);
        let mut decoder = FrameDecoder::new();

        // A byte at a time, which is the worst case and subsumes every other split.
        let mut events = Vec::new();
        for byte in &whole {
            events.extend(decoder.push(&[*byte]).expect("decodes"));
        }
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].name, "messageStop");
        assert!(!decoder.is_mid_frame(), "nothing should be left over");
    }

    /// Several frames very often arrive in one read, and a decoder that took only the first would
    /// drop the rest of the reply.
    #[test]
    fn several_frames_in_one_read_all_come_out() {
        let mut bytes = frame("a", br#"{"i":1}"#);
        bytes.extend(frame("b", br#"{"i":2}"#));
        bytes.extend(frame("c", br#"{"i":3}"#));

        let mut decoder = FrameDecoder::new();
        let events = decoder.push(&bytes).expect("decodes");
        assert_eq!(events.len(), 3);
        assert_eq!(events[2].name, "c");
        assert_eq!(events[2].payload, br#"{"i":3}"#);
    }

    /// The name is not always the first header, so every one has to be walked. Read from the first
    /// alone, every event would arrive unnamed and the whole reply would be dropped.
    #[test]
    fn the_name_is_found_behind_the_headers_in_front_of_it() {
        let mut headers = string_header(b":content-type", b"application/json");
        headers.extend(string_header(b":message-type", b"event"));
        headers.extend(string_header(EVENT_TYPE_HEADER, b"contentBlockDelta"));

        let mut decoder = FrameDecoder::new();
        let events = decoder
            .push(&frame_with_headers(br#"{"delta":{}}"#, &headers))
            .expect("decodes");
        assert_eq!(events[0].name, "contentBlockDelta");
    }

    /// A header whose value is not a string still has a width, and one skipped wrongly leaves the
    /// walk reading a value as the next header's name.
    #[test]
    fn a_header_that_is_not_a_string_is_skipped_by_its_own_width() {
        // A timestamp, which is eight bytes with no length in front of them.
        let mut headers = vec![b":date".len() as u8];
        headers.extend_from_slice(b":date");
        headers.push(8);
        headers.extend_from_slice(&0u64.to_be_bytes());
        headers.extend(string_header(EVENT_TYPE_HEADER, b"metadata"));

        let mut decoder = FrameDecoder::new();
        let events = decoder
            .push(&frame_with_headers(br#"{"usage":{}}"#, &headers))
            .expect("decodes");
        assert_eq!(events[0].name, "metadata");
    }

    /// A stream that ends mid-frame is a reply that was cut off. Without noticing, a truncated reply
    /// is returned as a whole one and the tool call the model was writing simply vanishes.
    #[test]
    fn an_incomplete_frame_is_reported_as_still_mid_frame() {
        let whole = frame("messageStop", br#"{}"#);
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
        let mut bytes = frame("messageStop", br#"{}"#);
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
        let mut bytes = frame("a", br#"{}"#);
        bytes[1] ^= 0xFF;

        let mut decoder = FrameDecoder::new();
        assert!(matches!(
            decoder.push(&bytes),
            Err(FrameError::Corrupt { .. })
        ));
    }

    /// The framing was sound, so the position in the stream is still known. A frame whose headers
    /// name no event is skipped rather than failing a reply that is otherwise arriving fine.
    #[test]
    fn a_frame_naming_no_event_is_skipped_not_fatal() {
        let mut bytes = frame_with_headers(br#"{"anything":true}"#, &[]);
        bytes.extend(frame_with_headers(
            br#"{"anything":true}"#,
            &string_header(b":message-type", b"event"),
        ));
        bytes.extend(frame("metadata", br#"{"usage":{}}"#));

        let mut decoder = FrameDecoder::new();
        let events = decoder.push(&bytes).expect("the framing was fine");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].name, "metadata");
    }

    /// Headers that run off the end of their own block name nothing, and a walk that read past them
    /// would be reading the payload as a header.
    #[test]
    fn a_truncated_header_block_names_nothing_rather_than_reading_on() {
        assert_eq!(event_name(&[5, b'a']), None);
        assert_eq!(event_name(&[1, b'a', STRING_VALUE, 0]), None);
        assert_eq!(event_name(&[1, b'a', STRING_VALUE, 0, 9, b'x']), None);
    }

    /// An empty read is what a quiet connection produces, and it must not be mistaken for an end.
    #[test]
    fn an_empty_read_yields_nothing_and_is_not_an_error() {
        let mut decoder = FrameDecoder::new();
        assert!(decoder.push(&[]).expect("decodes").is_empty());
        assert!(!decoder.is_mid_frame());
    }
}
