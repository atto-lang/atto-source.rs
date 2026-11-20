use oreslang_source_rs::layout_trace::{LayoutAccessTrace, LayoutHint, LayoutTraceError};

#[test]
fn llvm_compatible_access_shapes_remain_advisory() {
    let mut trace = LayoutAccessTrace::new(10, 3, false, false).unwrap();
    assert!(!trace.observe(0, 0));
    assert!(trace.configure(true));
    assert!(!trace.observe(10, 0));
    assert!(!trace.observe(0, 3));
    for row in 0..10 {
        assert!(trace.observe(row, 0));
    }
    assert_eq!(trace.samples(), 10);
    assert_eq!(trace.field_hits(0), Some(10));
    assert_eq!(trace.column_scan_transitions(), 9);
    assert_eq!(trace.hint(), LayoutHint::Soa);

    assert!(trace.configure(false));
    assert!(!trace.observe(0, 1));
    assert_eq!(trace.samples(), 10);
    assert!(trace.configure(true));
    trace.reset();
    for row in 0..10 {
        for field in 0..3 {
            assert!(trace.observe(row, field));
        }
    }
    assert_eq!(trace.samples(), 30);
    assert_eq!(trace.row_local_transitions(), 20);
    assert_eq!(trace.hint(), LayoutHint::Aos);
    assert_eq!(trace.field_hits(3), None);
}

#[test]
fn ties_and_short_traces_do_not_claim_layout_winner() {
    let mut trace = LayoutAccessTrace::new(10, 3, true, false).unwrap();
    assert!(trace.observe(0, 0));
    assert!(trace.observe(0, 1));
    assert!(trace.observe(1, 1));
    assert!(trace.observe(1, 2));
    assert!(trace.observe(2, 2));
    assert_eq!(trace.hint(), LayoutHint::Insufficient);
    trace.reset();
    assert_eq!(trace.samples(), 0);
    assert_eq!(trace.hint(), LayoutHint::Insufficient);
}

#[test]
fn sealed_profile_cannot_be_enabled_during_execution() {
    let mut disabled = LayoutAccessTrace::new(3, 2, false, true).unwrap();
    assert!(!disabled.configure(true));
    assert!(!disabled.observe(0, 0));
    assert_eq!(disabled.samples(), 0);

    let mut enabled = LayoutAccessTrace::new(3, 2, true, true).unwrap();
    assert!(!enabled.configure(false));
    assert!(enabled.enabled());
    assert!(enabled.observe(0, 0));
    assert_eq!(enabled.samples(), 1);
}

#[test]
fn large_row_edges_and_invalid_coordinates_do_not_poison_history() {
    assert!(matches!(
        LayoutAccessTrace::new(1, 0, true, false),
        Err(LayoutTraceError::InvalidFieldCount)
    ));
    assert!(matches!(
        LayoutAccessTrace::new(1, 65, true, false),
        Err(LayoutTraceError::InvalidFieldCount)
    ));
    let mut trace = LayoutAccessTrace::new(u64::MAX, 2, true, false).unwrap();
    assert!(trace.observe(u64::MAX - 3, 1));
    assert!(!trace.observe(u64::MAX, 1));
    assert!(!trace.observe(u64::MAX - 2, 2));
    assert!(trace.observe(u64::MAX - 2, 1));
    assert_eq!(trace.column_scan_transitions(), 1);
    assert!(trace.observe(u64::MAX - 1, 1));
    assert_eq!(trace.hint(), LayoutHint::Soa);
}
