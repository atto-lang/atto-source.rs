//! Arithmetic proof primitive for the candidate readonly Array<int> profile.
//! This is NOT array admission or a storage/ownership proof. A caller must
//! first bind the length interval to the exact validated receiver identity
//! and preserve that identity and length through mutation, calls and joins.
//! The existing runtime checks must remain until integrated guest evidence.
//! Candidate negative-index semantics follow Rust PR #178.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundKind { Index, SliceEndpoint }

/// Proves every independent combination of the inclusive signed index
/// interval and inclusive unsigned length interval satisfies the checked
/// normalization rule. Invalid input or insufficient proof returns false.
/// The candidate static descriptor profile is bounded at 1024 elements.
/// This does not establish physical byte extent, alignment or provenance.
pub fn proves_readonly_bound(
    index_lo: i64, index_hi: i64, len_lo: u32, len_hi: u32, kind: BoundKind,
) -> bool {
    if index_lo > index_hi || len_lo > len_hi || len_hi > 1024 {
        return false;
    }
    let n = i64::from(len_lo);
    if index_lo < -n { return false; }
    match kind {
        BoundKind::Index => index_hi < n,
        BoundKind::SliceEndpoint => index_hi <= n,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // Independent concrete model: evaluate signed normalization first, then
    // compare the normalized value to the actual length.
    fn concrete(index: i64, length: u32, kind: BoundKind) -> bool {
        let n = i128::from(length);
        let x = i128::from(index);
        let normalized = if x < 0 { x+n } else { x };
        normalized >= 0 && match kind {
            BoundKind::Index => normalized < n,
            BoundKind::SliceEndpoint => normalized <= n,
        }
    }

    #[test]
    fn interval_proof_matches_exhaustive_small_descriptors() {
        for lo in -12i64..=12 { for hi in lo..=12 {
            for len_lo in 0..=8 { for len_hi in len_lo..=8 {
                for kind in [BoundKind::Index, BoundKind::SliceEndpoint] {
                    let all=(lo..=hi).all(|x|
                        (len_lo..=len_hi).all(|n|concrete(x,n,kind)));
                    assert_eq!(proves_readonly_bound(lo,hi,len_lo,len_hi,kind),all);
                }
            }}
        }}
    }

    #[test]
    fn boundary_empty_negative_min_and_invalid_descriptors() {
        use BoundKind::*;
        assert!(!proves_readonly_bound(0,0,0,0,Index));
        assert!(proves_readonly_bound(0,0,0,0,SliceEndpoint));
        assert!(proves_readonly_bound(-4,3,4,8,Index));
        assert!(!proves_readonly_bound(-5,3,4,8,Index));
        assert!(!proves_readonly_bound(-4,4,4,8,Index));
        assert!(proves_readonly_bound(-4,4,4,8,SliceEndpoint));
        assert!(!proves_readonly_bound(i64::MIN,i64::MIN,1024,1024,Index));
        assert!(!proves_readonly_bound(i64::MAX,i64::MAX,1024,1024,SliceEndpoint));
        assert!(proves_readonly_bound(-1024,1023,1024,1024,Index));
        assert!(!proves_readonly_bound(0,0,0,1,Index));
        assert!(!proves_readonly_bound(2,1,4,4,Index));
        assert!(!proves_readonly_bound(0,0,4,3,Index));
        assert!(!proves_readonly_bound(0,0,1025,1025,Index));
        assert!(!proves_readonly_bound(0,0,1,u32::MAX,Index));
    }
}
