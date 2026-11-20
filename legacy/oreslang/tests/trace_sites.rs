use oreslang_source_rs::trace_sites::{TraceError, TraceEvent, TraceMode, TraceSites};
use std::num::NonZeroU64;
use std::sync::Arc;

fn threshold(value: u64) -> NonZeroU64 {
    NonZeroU64::new(value).unwrap()
}

#[test]
fn counts_per_site_and_crosses_threshold_once() {
    let sites = TraceSites::new(TraceMode::CountOnly, 2, threshold(3)).unwrap();
    assert_eq!(sites.record(0), Ok(TraceEvent::Counted));
    assert_eq!(sites.record(0), Ok(TraceEvent::Counted));
    assert_eq!(sites.record(0), Ok(TraceEvent::BecameHot));
    assert_eq!(sites.record(0), Ok(TraceEvent::Counted));
    assert_eq!(sites.record(1), Ok(TraceEvent::Counted));
    assert_eq!(sites.snapshot(), vec![4, 1]);
    assert_eq!(sites.is_hot(0), Ok(true));
    assert_eq!(sites.is_hot(1), Ok(false));
    assert_eq!(sites.record(2), Err(TraceError::SiteOutOfRange));
}

#[test]
fn strict_aot_allocates_no_trace_slots_and_records_no_events() {
    let sites = TraceSites::new(TraceMode::AotOnly, 0, threshold(1)).unwrap();
    assert_eq!(sites.record(42), Ok(TraceEvent::Disabled));
    assert_eq!(sites.snapshot(), Vec::<u64>::new());
    assert_eq!(sites.count(0), Err(TraceError::SiteOutOfRange));
}

#[test]
fn counter_count_and_storage_budget_are_bounded() {
    assert!(matches!(
        TraceSites::new(TraceMode::CountOnly, 0, threshold(1)),
        Err(TraceError::InvalidConfiguration)
    ));
    assert!(matches!(
        TraceSites::new(TraceMode::AotOnly, 1, threshold(1)),
        Err(TraceError::InvalidConfiguration)
    ));
    assert!(matches!(
        TraceSites::new(TraceMode::CountOnly, 4097, threshold(1)),
        Err(TraceError::InvalidConfiguration)
    ));
}

#[test]
fn contended_recording_is_monotonic_and_crosses_once() {
    let sites = Arc::new(TraceSites::new(TraceMode::CountOnly, 1, threshold(7)).unwrap());
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let sites = Arc::clone(&sites);
            std::thread::spawn(move || {
                let mut transitions = 0;
                for _ in 0..1000 {
                    if sites.record(0) == Ok(TraceEvent::BecameHot) {
                        transitions += 1;
                    }
                }
                transitions
            })
        })
        .collect();
    let transitions: usize = handles.into_iter().map(|h| h.join().unwrap()).sum();
    assert_eq!(transitions, 1);
    assert_eq!(sites.count(0), Ok(8000));
}
