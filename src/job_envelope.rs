//! Experimental **host-side** job envelope frame codec (wire revision 0).
//!
//! This module is not called by the Atto parser, checker, Wasm emitter, actor
//! mailbox or guest runtime. It does NOT implement source-level Job<T>,
//! guest-visible job IDs, admission receipts, ACKs, capability checks, durable
//! deduplication or a normative/protocol-stable ABI. It is pure, dependency-
//! free transport groundwork for a separately versioned guest ABI.
//!
//! All strings use exact UTF-8 bytes; do not normalize, case-fold or use a
//! delimiter-concatenated identity. The tenant/namespace still requires a
//! trusted capability check at both resolution and mailbox admission.

const MAGIC: &[u8; 4] = b"ATJB";
const VERSION: u8 = 0;
const HEADER_BYTES: usize = 28;
pub const MAX_IDENTITY_BYTES: usize = 4096;
pub const MAX_CORRELATION_BYTES: usize = 256;
pub const MAX_FRAME_BYTES: usize = HEADER_BYTES + MAX_IDENTITY_BYTES + MAX_CORRELATION_BYTES;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobEnvelope {
    pub namespace: String,
    pub job_id: String,
    pub attempt: u64,
    pub correlation_id: String,
    pub scalar_payload: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobFrameError {
    EmptyField,
    LimitExceeded,
    Truncated,
    InvalidMagic,
    UnsupportedVersion,
    InvalidFlags,
    InvalidUtf8,
    TrailingBytes,
}

fn check_lengths(namespace: usize, job: usize, correlation: usize) -> Result<(), JobFrameError> {
    if namespace == 0 || job == 0 || correlation == 0 {
        return Err(JobFrameError::EmptyField);
    }
    if namespace.checked_add(job).is_none_or(|n| n > MAX_IDENTITY_BYTES)
        || correlation > MAX_CORRELATION_BYTES
    {
        return Err(JobFrameError::LimitExceeded);
    }
    Ok(())
}

/// Deterministic little-endian frame:
/// magic[4] version[1] flags[1] namespace_len[2] job_len[2]
/// correlation_len[2] attempt[8] i64_payload[8] then exact UTF-8 bytes.
/// Revision 0 reserves all flag bits; non-zero flags must be rejected.
pub fn encode(job: &JobEnvelope) -> Result<Vec<u8>, JobFrameError> {
    let n = job.namespace.as_bytes();
    let j = job.job_id.as_bytes();
    let c = job.correlation_id.as_bytes();
    check_lengths(n.len(), j.len(), c.len())?;
    let mut frame = Vec::with_capacity(HEADER_BYTES + n.len() + j.len() + c.len());
    frame.extend_from_slice(MAGIC);
    frame.push(VERSION);
    frame.push(0); // exact flag encoding; no optional fields in this revision
    frame.extend_from_slice(&(n.len() as u16).to_le_bytes());
    frame.extend_from_slice(&(j.len() as u16).to_le_bytes());
    frame.extend_from_slice(&(c.len() as u16).to_le_bytes());
    frame.extend_from_slice(&job.attempt.to_le_bytes());
    frame.extend_from_slice(&job.scalar_payload.to_le_bytes());
    frame.extend_from_slice(n);
    frame.extend_from_slice(j);
    frame.extend_from_slice(c);
    debug_assert!(frame.len() <= MAX_FRAME_BYTES);
    Ok(frame)
}

pub fn decode(frame: &[u8]) -> Result<JobEnvelope, JobFrameError> {
    if frame.len() < HEADER_BYTES {
        return Err(JobFrameError::Truncated);
    }
    if &frame[..4] != MAGIC {
        return Err(JobFrameError::InvalidMagic);
    }
    if frame[4] != VERSION {
        return Err(JobFrameError::UnsupportedVersion);
    }
    if frame[5] != 0 {
        return Err(JobFrameError::InvalidFlags);
    }
    let n = u16::from_le_bytes([frame[6], frame[7]]) as usize;
    let j = u16::from_le_bytes([frame[8], frame[9]]) as usize;
    let c = u16::from_le_bytes([frame[10], frame[11]]) as usize;
    check_lengths(n, j, c)?;
    let expected = HEADER_BYTES.checked_add(n)
        .and_then(|v| v.checked_add(j))
        .and_then(|v| v.checked_add(c))
        .ok_or(JobFrameError::LimitExceeded)?;
    if expected > MAX_FRAME_BYTES {
        return Err(JobFrameError::LimitExceeded);
    }
    if frame.len() < expected {
        return Err(JobFrameError::Truncated);
    }
    if frame.len() > expected {
        return Err(JobFrameError::TrailingBytes);
    }
    let attempt = u64::from_le_bytes(frame[12..20].try_into()
        .expect("fixed header checked above"));
    let scalar_payload = i64::from_le_bytes(frame[20..28].try_into()
        .expect("fixed header checked above"));
    let ns_end = HEADER_BYTES + n;
    let id_end = ns_end + j;
    let namespace = std::str::from_utf8(&frame[HEADER_BYTES..ns_end])
        .map_err(|_| JobFrameError::InvalidUtf8)?;
    let job_id = std::str::from_utf8(&frame[ns_end..id_end])
        .map_err(|_| JobFrameError::InvalidUtf8)?;
    let correlation_id = std::str::from_utf8(&frame[id_end..expected])
        .map_err(|_| JobFrameError::InvalidUtf8)?;
    Ok(JobEnvelope {
        namespace: namespace.to_owned(),
        job_id: job_id.to_owned(),
        correlation_id: correlation_id.to_owned(),
        attempt,
        scalar_payload,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> JobEnvelope {
        JobEnvelope {
            namespace: "tenant-a".to_owned(),
            job_id: "invoice:42".to_owned(),
            correlation_id: "trace:1".to_owned(),
            attempt: 0,
            scalar_payload: 42,
        }
    }

    #[test]
    fn strict_scalar_and_attempt_roundtrip_at_boundaries() {
        for value in [i64::MIN, -1, 0, 1, i64::MAX] {
            for attempt in [0, 1, u64::MAX] {
                let mut job=sample();
                job.scalar_payload=value;
                job.attempt=attempt;
                let frame=encode(&job).unwrap();
                assert_eq!(decode(&frame),Ok(job));
                assert_eq!(&frame[..4],MAGIC);
                assert_eq!(frame[4],VERSION);
            }
        }
    }

    #[test]
    fn different_namespace_and_separator_encodings_do_not_alias() {
        let mut a=sample();
        a.namespace="a:b".to_owned();
        a.job_id="c".to_owned();
        let mut b=sample();
        b.namespace="a".to_owned();
        b.job_id="b:c".to_owned();
        assert_ne!(encode(&a),encode(&b));
        assert_eq!(decode(&encode(&a).unwrap()),Ok(a));
        assert_eq!(decode(&encode(&b).unwrap()),Ok(b));
    }

    #[test]
    fn unicode_nul_and_combining_codepoints_are_lossless() {
        for text in ["a\0b","é","e\u{301}","🧑‍💻","🚀"] {
            let mut job=sample();
            job.job_id=text.to_owned();
            assert_eq!(decode(&encode(&job).unwrap()),Ok(job));
        }
        let mut a=sample();
        let mut b=sample();
        a.job_id="é".into();
        b.job_id="e\u{301}".into();
        assert_ne!(encode(&a),encode(&b));
    }

    #[test]
    fn empty_keys_and_correlation_fail_closed() {
        let mut job=sample();
        for field in 0..3 {
            let old=job.clone();
            match field {
                0 => job.namespace.clear(),
                1 => job.job_id.clear(),
                _ => job.correlation_id.clear(),
            }
            assert_eq!(encode(&job),Err(JobFrameError::EmptyField));
            job=old;
        }
    }

    #[test]
    fn identity_budget_uses_utf8_bytes_and_accepts_boundary() {
        let mut job=sample();
        job.namespace="tenant".into();
        job.job_id="🚀".repeat((MAX_IDENTITY_BYTES-"tenant".len())/4);
        assert!(encode(&job).is_ok());
        job.job_id.push('🚀');
        assert_eq!(encode(&job),Err(JobFrameError::LimitExceeded));
    }

    #[test]
    fn correlation_budget_is_separate_from_namespace_and_id() {
        let mut job=sample();
        job.correlation_id="c".repeat(MAX_CORRELATION_BYTES);
        assert!(encode(&job).is_ok());
        job.correlation_id.push('c');
        assert_eq!(encode(&job),Err(JobFrameError::LimitExceeded));
    }

    #[test]
    fn every_truncated_header_is_rejected_without_panicking() {
        let frame=encode(&sample()).unwrap();
        for len in 0..HEADER_BYTES {
            assert_eq!(decode(&frame[..len]),Err(JobFrameError::Truncated));
        }
        for len in HEADER_BYTES..frame.len() {
            assert_eq!(decode(&frame[..len]),Err(JobFrameError::Truncated));
        }
    }

    #[test]
    fn magic_version_and_flag_must_match_exact_contract() {
        let original=encode(&sample()).unwrap();
        let mut frame=original.clone();
        frame[0]^=0x1;
        assert_eq!(decode(&frame),Err(JobFrameError::InvalidMagic));
        frame=original.clone();
        frame[4]=1;
        assert_eq!(decode(&frame),Err(JobFrameError::UnsupportedVersion));
        frame=original;
        frame[5]=1;
        assert_eq!(decode(&frame),Err(JobFrameError::InvalidFlags));
    }

    #[test]
    fn trailing_bytes_cannot_be_ignored_for_signatures_or_deduplication() {
        let mut frame=encode(&sample()).unwrap();
        frame.push(0);
        assert_eq!(decode(&frame),Err(JobFrameError::TrailingBytes));
    }

    #[test]
    fn forged_lengths_fail_before_content_can_be_dereferenced() {
        let mut frame=encode(&sample()).unwrap();
        frame[6..8].copy_from_slice(&u16::MAX.to_le_bytes());
        assert_eq!(decode(&frame),Err(JobFrameError::LimitExceeded));
        frame=encode(&sample()).unwrap();
        frame[10..12].copy_from_slice(&u16::MAX.to_le_bytes());
        assert_eq!(decode(&frame),Err(JobFrameError::LimitExceeded));
        frame=encode(&sample()).unwrap();
        frame[8..10].copy_from_slice(&0u16.to_le_bytes());
        assert_eq!(decode(&frame),Err(JobFrameError::EmptyField));
    }

    #[test]
    fn malformed_utf8_is_never_lossily_repaired() {
        let mut frame=encode(&sample()).unwrap();
        frame[HEADER_BYTES]=0xff;
        assert_eq!(decode(&frame),Err(JobFrameError::InvalidUtf8));
    }

    #[test]
    fn frames_are_deterministic_and_exactly_canonical() {
        let a=sample();
        assert_eq!(encode(&a),encode(&a));
        let b=encode(&a).unwrap();
        assert_eq!(encode(&decode(&b).unwrap()),Ok(b));
    }

    #[test]
    fn no_implicit_cross_tenant_or_attempt_coalescing() {
        let a=sample();
        let mut different_tenant=a.clone();
        different_tenant.namespace="tenant-b".into();
        let mut different_attempt=a.clone();
        different_attempt.attempt=1;
        let mut different_correlation=a.clone();
        different_correlation.correlation_id="trace:2".into();
        for other in [different_tenant,different_attempt,different_correlation] {
            assert_ne!(encode(&a),encode(&other));
        }
    }
}
