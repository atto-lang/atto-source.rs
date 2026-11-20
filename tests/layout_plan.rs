use oreslang_source_rs::{
    layout_plan::{FieldSpec, LayoutPlan},
    ref_layout::{LayoutKind, RefError},
};

const FIELDS: [FieldSpec; 2] = [
    FieldSpec { bytes: 3, align: 1 },
    FieldSpec { bytes: 8, align: 8 },
];

#[test]
fn aos_offsets_include_padding_and_aligned_rows() {
    let plan = LayoutPlan::new(LayoutKind::Aos, &FIELDS, 3).unwrap();
    assert_eq!(plan.total_bytes(), 48);
    assert_eq!(plan.field_offset(0, 0), Ok(0));
    assert_eq!(plan.field_offset(0, 1), Ok(8));
    assert_eq!(plan.field_offset(1, 0), Ok(16));
    assert_eq!(plan.field_offset(2, 1), Ok(40));
}

#[test]
fn soa_columns_are_dense_and_independently_aligned() {
    let plan = LayoutPlan::new(LayoutKind::Soa, &FIELDS, 3).unwrap();
    assert_eq!(plan.total_bytes(), 40);
    assert_eq!(plan.field_offset(2, 0), Ok(6));
    assert_eq!(plan.field_offset(0, 1), Ok(16));
    assert_eq!(plan.field_offset(2, 1), Ok(32));

    // Compact references in an SoA column need not inherit the 8-byte
    // alignment of heap objects. This is physical storage arithmetic only.
    let compact_ref = [FieldSpec { bytes: 4, align: 4 }];
    let plan = LayoutPlan::new(LayoutKind::Soa, &compact_ref, 3).unwrap();
    assert_eq!(plan.total_bytes(), 12);
    assert_eq!(plan.field_offset(2, 0), Ok(8));
}

#[test]
fn aosoa_reserves_partial_tail_block_and_checks_indices() {
    let plan = LayoutPlan::new(LayoutKind::Aosoa { lanes: 4 }, &FIELDS, 6).unwrap();
    assert_eq!(plan.total_bytes(), 96);
    assert_eq!(plan.field_offset(4, 0), Ok(48));
    assert_eq!(plan.field_offset(5, 1), Ok(72));
    assert_eq!(plan.field_offset(6, 0), Err(RefError::OutOfRange));
    assert_eq!(plan.field_offset(5, 2), Err(RefError::OutOfRange));
}

#[test]
fn all_supported_layouts_have_disjoint_field_storage() {
    let fields = [
        FieldSpec { bytes: 1, align: 1 },
        FieldSpec { bytes: 4, align: 4 },
        FieldSpec { bytes: 8, align: 8 },
    ];
    for kind in [
        LayoutKind::Aos,
        LayoutKind::Soa,
        LayoutKind::Aosoa { lanes: 1 },
        LayoutKind::Aosoa { lanes: 8 },
    ] {
        let plan = LayoutPlan::new(kind, &fields, 17).unwrap();
        let mut intervals = Vec::new();
        for row in 0..plan.row_count() {
            for (index, field) in fields.iter().enumerate() {
                let start = plan.field_offset(row, index).unwrap();
                let end = start + field.bytes;
                assert!(end <= plan.total_bytes());
                for &(other_start, other_end) in &intervals {
                    assert!(end <= other_start || start >= other_end, "{kind:?} overlaps storage");
                }
                intervals.push((start, end));
            }
        }
    }
}

#[test]
fn invalid_and_overflowing_plans_are_rejected() {
    assert_eq!(
        LayoutPlan::new(LayoutKind::Aos, &[], 1),
        Err(RefError::InvalidLayout)
    );
    assert_eq!(
        LayoutPlan::new(LayoutKind::Soa, &[FieldSpec { bytes: 0, align: 1 }], 1),
        Err(RefError::InvalidLayout)
    );
    assert_eq!(
        LayoutPlan::new(LayoutKind::Aos, &[FieldSpec { bytes: 1, align: 3 }], 1),
        Err(RefError::InvalidLayout)
    );
    assert_eq!(
        LayoutPlan::new(LayoutKind::Aosoa { lanes: 3 }, &FIELDS, 6),
        Err(RefError::InvalidLayout)
    );
    assert_eq!(
        LayoutPlan::new(LayoutKind::Aosoa { lanes: 2048 }, &FIELDS, 6),
        Err(RefError::InvalidLayout)
    );
    let many = [FieldSpec { bytes: 1, align: 1 }; 65];
    assert_eq!(
        LayoutPlan::new(LayoutKind::Soa, &many, 1),
        Err(RefError::InvalidLayout)
    );
    let large = [FieldSpec { bytes: 8, align: 8 }];
    assert_eq!(
        LayoutPlan::new(LayoutKind::Aos, &large, usize::MAX),
        Err(RefError::Overflow)
    );
    assert_eq!(
        LayoutPlan::new(LayoutKind::Soa, &large, usize::MAX),
        Err(RefError::Overflow)
    );
    assert_eq!(
        LayoutPlan::new(LayoutKind::Aosoa { lanes: 8 }, &large, usize::MAX),
        Err(RefError::Overflow)
    );
}

#[test]
fn zero_rows_produce_no_guest_storage_and_cannot_be_indexed() {
    for kind in [
        LayoutKind::Aos,
        LayoutKind::Soa,
        LayoutKind::Aosoa { lanes: 8 },
    ] {
        let plan = LayoutPlan::new(kind, &FIELDS, 0).unwrap();
        assert_eq!(plan.total_bytes(), 0);
        assert_eq!(plan.field_count(), FIELDS.len());
        assert_eq!(plan.field_offset(0, 0), Err(RefError::OutOfRange));
    }
}

#[test]
fn java_compatible_1024_lane_tile_tail_has_exact_offsets() {
    let fields = [FieldSpec { bytes: 8, align: 8 }; 2];
    let tile = LayoutPlan::new(LayoutKind::Aosoa { lanes: 1024 }, &fields, 1025).unwrap();
    assert_eq!(tile.total_bytes(), 32768);
    assert_eq!(tile.field_offset(1023, 0), Ok(8184));
    assert_eq!(tile.field_offset(1024, 0), Ok(16384));
    assert_eq!(tile.field_offset(1024, 1), Ok(24576));
    assert_eq!(tile.field_offset(1025, 0), Err(RefError::OutOfRange));

    let aos = LayoutPlan::new(LayoutKind::Aos, &fields, 1025).unwrap();
    let soa = LayoutPlan::new(LayoutKind::Soa, &fields, 1025).unwrap();
    assert_eq!(aos.total_bytes(), 16400);
    assert_eq!(soa.total_bytes(), 16400);
    assert_eq!(aos.field_offset(1024, 0), Ok(16384));
    assert_eq!(soa.field_offset(1024, 0), Ok(8192));
    assert_eq!(soa.field_offset(1024, 1), Ok(16392));

    assert_eq!(
        LayoutPlan::new(LayoutKind::Aosoa { lanes: 2048 }, &fields, 1025),
        Err(RefError::InvalidLayout)
    );
}

#[test]
fn checked_field_spans_agree_with_offsets_and_enforce_backing_bounds() {
    for kind in [
        LayoutKind::Aos,
        LayoutKind::Soa,
        LayoutKind::Aosoa { lanes: 1 },
        LayoutKind::Aosoa { lanes: 4 },
        LayoutKind::Aosoa { lanes: 1024 },
    ] {
        let plan = LayoutPlan::new(kind, &FIELDS, 1025).unwrap();
        for row in [0, 1, 3, 4, 1023, 1024] {
            for (field_index, field) in FIELDS.iter().enumerate() {
                let start = plan.field_offset(row, field_index).unwrap();
                let expected_end = start.checked_add(field.bytes).unwrap();
                let span = plan
                    .checked_field_span(row, field_index, plan.total_bytes())
                    .unwrap();
                assert_eq!(span, start..expected_end, "{kind:?}, row={row}");
                assert_eq!(
                    plan.checked_field_span(row, field_index, plan.total_bytes() - 1),
                    Err(RefError::OutOfRange),
                    "{kind:?}, row={row} must reject an incomplete owned store"
                );
                assert_eq!(
                    plan.checked_field_span(row, field_index, plan.total_bytes()),
                    Ok(start..expected_end),
                    "{kind:?}, row={row} complete backing must be admitted"
                );
            }
        }
        assert_eq!(
            plan.checked_field_span(plan.row_count(), 0, plan.total_bytes()),
            Err(RefError::OutOfRange)
        );
        assert_eq!(
            plan.checked_field_span(0, FIELDS.len(), plan.total_bytes()),
            Err(RefError::OutOfRange)
        );
    }
}

#[test]
fn empty_layout_never_admits_a_field_span() {
    for kind in [LayoutKind::Aos, LayoutKind::Soa, LayoutKind::Aosoa { lanes: 8 }] {
        let plan = LayoutPlan::new(kind, &FIELDS, 0).unwrap();
        assert_eq!(plan.total_bytes(), 0);
        assert_eq!(
            plan.checked_field_span(0, 0, usize::MAX),
            Err(RefError::OutOfRange)
        );
    }
}
