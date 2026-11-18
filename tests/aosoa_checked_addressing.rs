use oreslang_source_rs::ref_layout::{
    aosoa_field_offset, checked_aosoa_field_span, RefError,
};

fn span(row: usize, row_count: usize, bytes: usize) -> Result<std::ops::Range<usize>, RefError> {
    checked_aosoa_field_span(row, row_count, 8, 192, 32, 4, 4, bytes)
}

#[test]
fn canonical_aosoa_span_matches_existing_index_arithmetic() {
    for row in 0..16 {
        let result = span(row, 16, 384).unwrap();
        let raw = aosoa_field_offset(row, 8, 192, 32, 4).unwrap();
        assert_eq!(result, raw..raw + 4);
    }
}

#[test]
fn partial_last_block_honors_logical_row_count() {
    assert_eq!(span(9, 10, 232), Ok(228..232));
    assert_eq!(span(10, 10, 384), Err(RefError::OutOfRange));
}

#[test]
fn no_rows_is_always_out_of_range() {
    assert_eq!(span(0, 0, 384), Err(RefError::OutOfRange));
}

#[test]
fn backing_store_must_cover_entire_field_access() {
    assert_eq!(span(9, 10, 231), Err(RefError::OutOfRange));
    assert_eq!(span(9, 10, 232), Ok(228..232));
}

#[test]
fn reject_field_that_overlaps_next_block() {
    assert_eq!(
        checked_aosoa_field_span(0, 8, 8, 20, 0, 4, 4, 1000),
        Err(RefError::InvalidLayout)
    );
}

#[test]
fn reject_overlapping_field_elements() {
    assert_eq!(
        checked_aosoa_field_span(0, 8, 8, 192, 0, 2, 4, 192),
        Err(RefError::InvalidLayout)
    );
}

#[test]
fn reject_invalid_or_zero_layout_parameters() {
    for (lanes, block_stride, element_stride, width) in [
        (0, 192, 4, 4),
        (3, 192, 4, 4),
        (8, 0, 4, 4),
        (8, 192, 0, 4),
        (8, 192, 4, 0),
    ] {
        assert_eq!(
            checked_aosoa_field_span(
                0, 8, lanes, block_stride, 0, element_stride, width, 1000
            ),
            Err(RefError::InvalidLayout)
        );
    }
}

#[test]
fn exact_block_boundary_is_allowed() {
    assert_eq!(
        checked_aosoa_field_span(7, 8, 8, 32, 0, 4, 4, 32),
        Ok(28..32)
    );
}

#[test]
fn field_base_or_lane_arithmetic_overflow_is_rejected() {
    assert_eq!(
        checked_aosoa_field_span(0, 8, 8, 32, usize::MAX, 4, 4, usize::MAX),
        Err(RefError::Overflow)
    );
}

#[test]
fn repeated_block_stride_overflow_is_rejected() {
    assert_eq!(
        checked_aosoa_field_span(
            usize::MAX - 1,
            usize::MAX,
            8,
            usize::MAX,
            0,
            1,
            1,
            usize::MAX,
        ),
        Err(RefError::Overflow)
    );
}

#[test]
fn allocation_length_is_not_conflated_with_row_count() {
    assert_eq!(span(0, 16, 0), Err(RefError::OutOfRange));
    assert_eq!(span(15, 16, 384), Ok(252..256));
}

#[test]
fn all_admitted_spans_fit_backing_and_do_not_cross_blocks() {
    for lanes in [1usize, 2, 4, 8, 16, 32] {
        let block_stride = lanes * 4;
        let rows = lanes * 3 + 1;
        let bytes = block_stride * 4;
        for row in 0..rows {
            let range = checked_aosoa_field_span(
                row,
                rows,
                lanes,
                block_stride,
                0,
                4,
                4,
                bytes,
            )
            .unwrap();
            assert!(range.start < range.end);
            assert!(range.end <= bytes);
            let first_block_byte = (row / lanes) * block_stride;
            assert!(range.start >= first_block_byte);
            assert!(range.end <= first_block_byte + block_stride);
        }
    }
}

#[test]
fn shared_heterogeneous_aosoa_v1_golden_offsets_match() {
    // Matches oreslang-platform-conformance:
    // contracts/v1/heterogeneous-four-layout-golden-v1.json (AoSoA plan).
    // Reference-only physical offsets; not native guest execution.
    let golden = [
        (0, 4, 4, [0, 4, 8, 12, 72]),
        (16, 8, 8, [16, 24, 32, 40, 88]),
        (48, 1, 1, [48, 49, 50, 51, 120]),
        (52, 4, 4, [52, 56, 60, 64, 124]),
    ];
    for (field_base, field_stride, field_width, expected) in golden {
        for (row, expected_start) in expected.into_iter().enumerate() {
            let range = checked_aosoa_field_span(
                row,
                5,
                4,
                72,
                field_base,
                field_stride,
                field_width,
                144,
            )
            .unwrap();
            assert_eq!(range, expected_start..expected_start + field_width);
        }
    }
}

#[test]
fn canonical_tile_width_bounds_deny_oversized_power_of_two() {
    // oreslang-platform-conformance/contracts/v1/data-layout-abi-v1.json
    // specifies power-of-two tiles with a maximum of 1024 lanes.
    assert_eq!(
        checked_aosoa_field_span(0, 2048, 2048, 8192, 0, 4, 4, 8192),
        Err(RefError::InvalidLayout)
    );
    assert_eq!(
        checked_aosoa_field_span(1023, 1024, 1024, 4096, 0, 4, 4, 4096),
        Ok(4092..4096)
    );
}
