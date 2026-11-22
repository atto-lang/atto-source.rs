//! AOT-first static numeric probe registry and actor-owned fixed counter bank.
//!
//! Compiler/runtime policy reference only. Does not instrument executable guest
//! Wasm or authorize a runtime JIT. No heap, clock, logging or I/O on record().
pub const PROBE_IDS: [u32; 7] = [23, 24, 25, 26, 27, 28, 29];

#[inline]
pub const fn slot_for_probe(id: u32) -> Option<usize> {
    match id {
        23 => Some(0), 24 => Some(1), 25 => Some(2), 26 => Some(3),
        27 => Some(4), 28 => Some(5), 29 => Some(6),
        _ => None,
    }
}

#[inline]
pub fn should_sample(key: i64, every_n: i64) -> bool {
    if key < 0 || !(1..=65_536).contains(&every_n) {
        return false;
    }
    key % every_n == 0
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordOutcome {
    Disabled,
    Unsampled,
    UnknownProbe,
    Recorded,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PerfCounterBank {
    enabled: bool,
    counts: [u64; 7],
}

impl PerfCounterBank {
    pub const fn new(enabled: bool) -> Self {
        Self { enabled, counts: [0; 7] }
    }

    /// O(1) actor-local counter hook: no clock reads, allocations, formatting,
    /// atomics, locks, I/O, runnable tasks, guest code or imports.
    /// A disabled sink must not sample or resolve the probe ID.
    pub fn record(&mut self, probe: u32, sample_key: i64, every_n: i64) -> RecordOutcome {
        if !self.enabled {
            return RecordOutcome::Disabled;
        }
        if !should_sample(sample_key, every_n) {
            return RecordOutcome::Unsampled;
        }
        let Some(slot) = slot_for_probe(probe) else {
            return RecordOutcome::UnknownProbe;
        };
        self.counts[slot] = self.counts[slot].saturating_add(1);
        RecordOutcome::Recorded
    }

    pub fn count(&self, probe: u32) -> Option<u64> {
        slot_for_probe(probe).map(|slot| self.counts[slot])
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_is_exact_and_no_renumbering() {
        assert_eq!(PROBE_IDS, [23, 24, 25, 26, 27, 28, 29]);
        for (slot, id) in PROBE_IDS.iter().copied().enumerate() {
            assert_eq!(slot_for_probe(id), Some(slot));
        }
        for id in [0, 1, 22, 30, u32::MAX] {
            assert_eq!(slot_for_probe(id), None);
        }
    }

    #[test]
    fn sample_guards_zero_negative_and_large_divisors() {
        for divisor in 1..=65_536i64 {
            assert_eq!(should_sample(i64::MAX, divisor),
                       i64::MAX % divisor == 0);
        }
        for (key, divisor) in [(-1, 1), (0, 0), (1, -1), (0, 65_537)] {
            assert!(!should_sample(key, divisor));
        }
        assert!(should_sample(42, 7));
        assert!(!should_sample(43, 7));
    }

    #[test]
    fn disabled_and_invalid_hooks_do_not_mutate_counters() {
        let mut b = PerfCounterBank::new(false);
        assert_eq!(b.record(23, 42, 1), RecordOutcome::Disabled);
        assert_eq!(b.count(23), Some(0));
        b.set_enabled(true);
        assert_eq!(b.record(23, 42, 0), RecordOutcome::Unsampled);
        assert_eq!(b.record(30, 42, 1), RecordOutcome::UnknownProbe);
        assert_eq!(b.record(23, 42, 1), RecordOutcome::Recorded);
        assert_eq!(b.count(23), Some(1));
        assert_eq!(b.count(24), Some(0));
        assert_eq!(b.count(30), None);
    }

    #[test]
    fn counter_saturates_without_wraparound_or_duplicate_hot_event() {
        let mut b = PerfCounterBank::new(true);
        b.counts[0] = u64::MAX - 1;
        assert_eq!(b.record(23, 42, 1), RecordOutcome::Recorded);
        assert_eq!(b.record(23, 42, 1), RecordOutcome::Recorded);
        assert_eq!(b.count(23), Some(u64::MAX));
    }
}
