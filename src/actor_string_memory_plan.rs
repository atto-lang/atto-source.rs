//! Checked layout plan for a future ActorMail<string> Wasm ABI.
//!
//! This module does NOT enable compiled string actors. The current production
//! checker still rejects ActorMail<string>. An independent host must verify a
//! compiled artifact's memory sections and trusted compiler metadata before
//! adopting any plan. Never trust a guest-provided scratch region declaration.
#![forbid(unsafe_code)]

pub const PAGE_BYTES: u32 = 65_536;
pub const STRING_MAIL_PAGES: u32 = 1;
pub const STRING_MAIL_MAX_BYTES: u32 = PAGE_BYTES;
pub const SCALAR_BASE_PAGES: u32 = 1;
pub const HEAP_BASE_PAGES: u32 = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StringMailLayoutError {
    UnsupportedGuestMemoryProfile,
    DataBeyondMemory,
    InvalidHeapRange,
    InvalidReservation,
    InvalidMemoryLimits,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StringMailLayout {
    /// Existing source+heap memory footprint in pages, unchanged.
    pub ordinary_pages: u32,
    /// Exact min=max Wasm32 memory page count when this ABI is admitted.
    pub fixed_pages: u32,
    /// Start of a private host-mail scratch page, never in the ordinary heap.
    pub scratch_offset: u32,
    pub scratch_capacity: u32,
}

impl StringMailLayout {
    /// A compiled bytecode reader must validate the real Wasm memory limits,
    /// data segments, and heap ceiling independently of the guest's claims.
    pub fn verify_emitted_memory(
        self,
        emitted_min_pages: u32,
        emitted_max_pages: u32,
        actual_data_end: u64,
        ordinary_heap_ceiling: u64,
    ) -> Result<(), StringMailLayoutError> {
        if emitted_min_pages != self.fixed_pages ||
            emitted_max_pages != self.fixed_pages ||
            self.fixed_pages != self.ordinary_pages + STRING_MAIL_PAGES
        {
            return Err(StringMailLayoutError::InvalidMemoryLimits);
        }
        let start = u64::from(self.ordinary_pages) * u64::from(PAGE_BYTES);
        if u64::from(self.scratch_offset) != start ||
            self.scratch_capacity != STRING_MAIL_MAX_BYTES
        {
            return Err(StringMailLayoutError::InvalidReservation);
        }
        if actual_data_end > start {
            return Err(StringMailLayoutError::DataBeyondMemory);
        }
        if ordinary_heap_ceiling < actual_data_end || ordinary_heap_ceiling > start {
            return Err(StringMailLayoutError::InvalidHeapRange);
        }
        Ok(())
    }
}

/// Compute an append-only one-page mailbox reservation that is separated
/// from the compiler's already-bounded ordinary Wasm allocation region.
/// Existing scalar-only/no-string module memory must remain byte-identical.
pub fn plan_string_mail_memory(
    receives_string: bool,
    ordinary_pages: u32,
    static_data_end: u64,
    ordinary_heap_ceiling: u64,
) -> Result<Option<StringMailLayout>, StringMailLayoutError> {
    if !receives_string {
        return Ok(None);
    }
    if ordinary_pages != SCALAR_BASE_PAGES && ordinary_pages != HEAP_BASE_PAGES {
        return Err(StringMailLayoutError::UnsupportedGuestMemoryProfile);
    }
    let end = u64::from(ordinary_pages) * u64::from(PAGE_BYTES);
    if static_data_end > end {
        return Err(StringMailLayoutError::DataBeyondMemory);
    }
    if ordinary_heap_ceiling < static_data_end || ordinary_heap_ceiling > end {
        return Err(StringMailLayoutError::InvalidHeapRange);
    }
    let layout = StringMailLayout {
        ordinary_pages,
        fixed_pages: ordinary_pages + STRING_MAIL_PAGES,
        scratch_offset: end as u32,
        scratch_capacity: STRING_MAIL_MAX_BYTES,
    };
    layout.verify_emitted_memory(
        layout.fixed_pages, layout.fixed_pages, static_data_end, ordinary_heap_ceiling
    )?;
    Ok(Some(layout))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unmodified_scalar_and_heap_programs_need_no_new_memory() {
        assert_eq!(plan_string_mail_memory(false, 1, 0, 0), Ok(None));
        assert_eq!(plan_string_mail_memory(false, 16, 65_536, 1_048_576), Ok(None));
    }

    #[test]
    fn scalar_string_actor_reserves_second_fixed_page() {
        let plan = plan_string_mail_memory(true, 1, 10_000, 64_000)
            .unwrap().unwrap();
        assert_eq!(plan, StringMailLayout {
            ordinary_pages: 1, fixed_pages: 2,
            scratch_offset: 65_536, scratch_capacity: 65_536,
        });
        assert_eq!(plan.verify_emitted_memory(2, 2, 10_000, 64_000), Ok(()));
    }

    #[test]
    fn actor_with_checked_heap_reserves_seventeenth_fixed_page() {
        let plan = plan_string_mail_memory(true, 16, 50_000, 1_048_576)
            .unwrap().unwrap();
        assert_eq!(plan, StringMailLayout {
            ordinary_pages: 16, fixed_pages: 17,
            scratch_offset: 1_048_576, scratch_capacity: 65_536,
        });
    }

    #[test]
    fn arbitrary_or_unbounded_guest_profiles_are_never_silently_accepted() {
        for pages in [0, 2, 15, 17, 65_535, u32::MAX] {
            assert_eq!(plan_string_mail_memory(true, pages, 0, 0),
                Err(StringMailLayoutError::UnsupportedGuestMemoryProfile));
        }
    }

    #[test]
    fn data_segment_must_not_overlap_reserved_page() {
        assert_eq!(plan_string_mail_memory(true, 1, 65_537, 65_537),
            Err(StringMailLayoutError::DataBeyondMemory));
        assert_eq!(plan_string_mail_memory(true, 16, 1_048_577, 1_048_577),
            Err(StringMailLayoutError::DataBeyondMemory));
    }

    #[test]
    fn ordinary_heap_may_not_alias_reserved_region_or_precede_data() {
        assert_eq!(plan_string_mail_memory(true, 1, 1000, 999),
            Err(StringMailLayoutError::InvalidHeapRange));
        assert_eq!(plan_string_mail_memory(true, 1, 1000, 65_537),
            Err(StringMailLayoutError::InvalidHeapRange));
    }

    #[test]
    fn mismatched_compiled_memory_limits_are_rejected_by_host() {
        let p = plan_string_mail_memory(true, 1, 0, 65_536).unwrap().unwrap();
        for (min, max) in [(1, 2), (2, 3), (2, 1), (3, 3), (0, 2)] {
            assert_eq!(p.verify_emitted_memory(min, max, 0, 65_536),
                Err(StringMailLayoutError::InvalidMemoryLimits));
        }
    }

    #[test]
    fn forged_reservation_offset_or_capacity_is_rejected() {
        let mut p = plan_string_mail_memory(true, 1, 0, 65_536).unwrap().unwrap();
        p.scratch_offset = 0;
        assert_eq!(p.verify_emitted_memory(2, 2, 0, 65_536),
            Err(StringMailLayoutError::InvalidReservation));
        p.scratch_offset = 65_536;
        p.scratch_capacity = 65_535;
        assert_eq!(p.verify_emitted_memory(2, 2, 0, 65_536),
            Err(StringMailLayoutError::InvalidReservation));
    }

    #[test]
    fn host_must_recheck_real_data_and_heap_ceiling() {
        let p = plan_string_mail_memory(true, 1, 0, 65_536).unwrap().unwrap();
        assert_eq!(p.verify_emitted_memory(2, 2, 65_537, 65_537),
            Err(StringMailLayoutError::DataBeyondMemory));
        assert_eq!(p.verify_emitted_memory(2, 2, 0, 65_537),
            Err(StringMailLayoutError::InvalidHeapRange));
    }

    #[test]
    fn valid_data_and_heap_thresholds_never_overlap_scratch() {
        for ordinary_pages in [1u32, 16] {
            let end = u64::from(ordinary_pages) * u64::from(PAGE_BYTES);
            for static_end in [0, 1, end / 2, end] {
                let layout = plan_string_mail_memory(true, ordinary_pages, static_end, end)
                    .unwrap().unwrap();
                assert_eq!(layout.verify_emitted_memory(
                    layout.fixed_pages, layout.fixed_pages, static_end, end
                ), Ok(()));
                assert_eq!(u64::from(layout.scratch_offset), end);
            }
        }
    }
}
