//! Candidate bounded UTF-8 actor DATA payload framing.
//!
//! This is a host-independent **codec**, not a guest messaging implementation.
//! The existing Wasm actor ABI still admits only int/bool; the compiler must
//! reject ActorMail<string> until a checked per-worker memory copy, mailbox
//! admission and guest decoder are implemented and independently executed.
//! Decoded bytes grant no ActorRef, tenant authority or CONTROL privileges.
#![forbid(unsafe_code)]

const MAGIC: &[u8; 4] = b"ATAM";
const VERSION: u8 = 1;
const STRING_DATA: u8 = 1;
const HEADER_BYTES: usize = 10;
pub const MAX_STRING_BYTES: usize = 64 * 1024;
pub const MAX_FRAME_BYTES: usize = HEADER_BYTES + MAX_STRING_BYTES;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StringMailError {
    Truncated,
    TrailingBytes,
    LimitExceeded,
    UnsupportedMagic,
    UnsupportedVersion,
    InvalidPayloadKind,
    InvalidUtf8,
    AllocationFailure,
}

/// Encode one application DATA string in a deterministic length-delimited
/// envelope: "ATAM" | version:u8 | kind:u8 | byte_len:u32-le | utf8[byte_len].
/// The payload length is measured in bytes, never Unicode scalar values.
pub fn encode_string(payload: &str) -> Result<Vec<u8>, StringMailError> {
    let bytes = payload.as_bytes();
    if bytes.len() > MAX_STRING_BYTES {
        return Err(StringMailError::LimitExceeded);
    }
    let frame_len = HEADER_BYTES
        .checked_add(bytes.len())
        .ok_or(StringMailError::LimitExceeded)?;
    let mut frame = Vec::new();
    frame.try_reserve_exact(frame_len)
        .map_err(|_| StringMailError::AllocationFailure)?;
    frame.extend_from_slice(MAGIC);
    frame.push(VERSION);
    frame.push(STRING_DATA);
    frame.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    frame.extend_from_slice(bytes);
    Ok(frame)
}

/// Validate the exact frame before cloning the message into owned host memory.
/// Unknown kind tags (including any purported supervisor CONTROL tag) always
/// fail, rather than being interpreted as ordinary application DATA.
pub fn decode_string(frame: &[u8]) -> Result<String, StringMailError> {
    if frame.len() < HEADER_BYTES {
        return Err(StringMailError::Truncated);
    }
    if frame.len() > MAX_FRAME_BYTES {
        return Err(StringMailError::LimitExceeded);
    }
    if &frame[..MAGIC.len()] != MAGIC {
        return Err(StringMailError::UnsupportedMagic);
    }
    if frame[4] != VERSION {
        return Err(StringMailError::UnsupportedVersion);
    }
    if frame[5] != STRING_DATA {
        return Err(StringMailError::InvalidPayloadKind);
    }
    let declared = u32::from_le_bytes(frame[6..10].try_into()
        .expect("complete header already checked")) as usize;
    if declared > MAX_STRING_BYTES {
        return Err(StringMailError::LimitExceeded);
    }
    let expected = HEADER_BYTES
        .checked_add(declared)
        .ok_or(StringMailError::LimitExceeded)?;
    if frame.len() < expected {
        return Err(StringMailError::Truncated);
    }
    if frame.len() > expected {
        return Err(StringMailError::TrailingBytes);
    }
    let payload = std::str::from_utf8(&frame[HEADER_BYTES..])
        .map_err(|_| StringMailError::InvalidUtf8)?;
    // Avoid accepting an incomplete buffer while reading a string as borrowed
    // memory in a foreign worker. The owned value must outlive frame storage.
    Ok(payload.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_ascii_and_multibyte_roundtrip() {
        for s in ["", "hello", "a\0b", "é", "e\u{301}", "🦀", "🧑‍💻"] {
            let frame = encode_string(s).unwrap();
            assert_eq!(&frame[..4], b"ATAM");
            assert_eq!(frame[4], 1);
            assert_eq!(frame[5], 1);
            assert_eq!(decode_string(&frame), Ok(s.to_owned()));
            assert_eq!(u32::from_le_bytes(frame[6..10].try_into().unwrap()) as usize,
                s.len());
        }
    }

    #[test]
    fn distinct_unicode_forms_and_nul_are_not_normalized() {
        let composed = encode_string("é").unwrap();
        let decomposed = encode_string("e\u{301}").unwrap();
        assert_ne!(composed, decomposed);
        assert_eq!(decode_string(&encode_string("x\0y").unwrap()), Ok("x\0y".into()));
    }

    #[test]
    fn exact_byte_quota_and_one_byte_overflow() {
        let max = "x".repeat(MAX_STRING_BYTES);
        let frame = encode_string(&max).unwrap();
        assert_eq!(frame.len(), MAX_FRAME_BYTES);
        assert_eq!(decode_string(&frame), Ok(max.clone()));
        assert_eq!(encode_string(&(max + "x")), Err(StringMailError::LimitExceeded));
    }

    #[test]
    fn utf8_quota_counts_bytes_not_codepoints() {
        let accepted = "🚀".repeat(MAX_STRING_BYTES / 4);
        assert!(encode_string(&accepted).is_ok());
        assert_eq!(encode_string(&(accepted + "🚀")), Err(StringMailError::LimitExceeded));
    }

    #[test]
    fn all_truncated_prefixes_fail() {
        let frame = encode_string("bounded payload").unwrap();
        for n in 0..frame.len() {
            assert_eq!(decode_string(&frame[..n]), Err(StringMailError::Truncated),
                "prefix length {n}");
        }
    }

    #[test]
    fn extra_bytes_are_not_ignored() {
        let mut frame = encode_string("ok").unwrap();
        frame.push(0);
        assert_eq!(decode_string(&frame), Err(StringMailError::TrailingBytes));
    }

    #[test]
    fn oversized_frames_fail_before_decoding() {
        let frame = vec![0; MAX_FRAME_BYTES + 1];
        assert_eq!(decode_string(&frame), Err(StringMailError::LimitExceeded));
    }

    #[test]
    fn attacker_controlled_length_fails_closed() {
        let mut frame = encode_string("ok").unwrap();
        frame[6..10].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(decode_string(&frame), Err(StringMailError::LimitExceeded));
        frame[6..10].copy_from_slice(&1u32.to_le_bytes());
        assert_eq!(decode_string(&frame), Err(StringMailError::TrailingBytes));
    }

    #[test]
    fn forged_magic_version_and_control_kind_fail() {
        let original = encode_string("ok").unwrap();
        let mut frame = original.clone();
        frame[0] = b'X';
        assert_eq!(decode_string(&frame), Err(StringMailError::UnsupportedMagic));
        frame = original.clone();
        frame[4] = 2;
        assert_eq!(decode_string(&frame), Err(StringMailError::UnsupportedVersion));
        for kind in [0, 2, 255] {
            frame = original.clone();
            frame[5] = kind;
            assert_eq!(decode_string(&frame), Err(StringMailError::InvalidPayloadKind));
        }
    }

    #[test]
    fn malformed_utf8_is_not_lossily_repaired() {
        let mut frame = encode_string("ok").unwrap();
        frame[HEADER_BYTES] = 0xff;
        assert_eq!(decode_string(&frame), Err(StringMailError::InvalidUtf8));
        frame = encode_string("ok").unwrap();
        frame[HEADER_BYTES] = 0xc0;
        assert_eq!(decode_string(&frame), Err(StringMailError::InvalidUtf8));
    }

    #[test]
    fn decoding_owns_message_memory_independent_of_frame() {
        let mut frame = encode_string("hello").unwrap();
        let decoded = decode_string(&frame).unwrap();
        frame.fill(0);
        assert_eq!(decoded, "hello");
    }

    #[test]
    fn byte_identical_canonical_reencoding() {
        for s in ["", "hello", "é", "a\0b"] {
            let b = encode_string(s).unwrap();
            assert_eq!(encode_string(&decode_string(&b).unwrap()), Ok(b));
        }
    }
}
