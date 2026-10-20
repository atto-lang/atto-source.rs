//! Domain-relative managed references and physical layout arithmetic.
//! No native pointer or ActorRef is represented by this type.
use std::num::NonZeroU32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefMode {
    Compressed32,
    Native,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutKind {
    Aos,
    Soa,
    Aosoa { lanes: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefError {
    BadAlignment,
    OutOfRange,
    InvalidDomain,
    InvalidLayout,
    Overflow,
}

/// A zero payload means null. All nonzero payloads encode an aligned
/// byte offset divided by 2^shift, plus one. The first object may be at offset 0.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompressedRef(u32);

impl CompressedRef {
    pub const NULL: Self = Self(0);

    pub fn is_null(self) -> bool {
        self.0 == 0
    }

    pub fn raw(self) -> u32 {
        self.0
    }

    pub fn from_raw(raw: u32) -> Self {
        Self(raw)
    }
}

/// Heap domain membership must be proven separately by the allocator/borrow checker.
/// base and length are native-width addresses/capacities, not serialized data.
#[derive(Debug, Clone, Copy)]
pub struct Region {
    pub base: usize,
    pub byte_len: usize,
    pub shift: u8,
}

impl Region {
    pub fn new(base: usize, byte_len: usize, shift: u8) -> Result<Self, RefError> {
        if shift > 15 {
            return Err(RefError::BadAlignment);
        }
        let align = 1usize << shift;
        if !base.is_multiple_of(align) {
            return Err(RefError::BadAlignment);
        }
        base.checked_add(byte_len).ok_or(RefError::Overflow)?;
        Ok(Self {
            base,
            byte_len,
            shift,
        })
    }

    pub fn encode_offset(self, offset: usize) -> Result<CompressedRef, RefError> {
        let align = 1usize << self.shift;
        if !offset.is_multiple_of(align) {
            return Err(RefError::BadAlignment);
        }
        if offset >= self.byte_len {
            return Err(RefError::OutOfRange);
        }
        let slot = offset >> self.shift;
        // Reserve zero for null; last representable slot is 2^32-2.
        let slot: u32 = slot.try_into().map_err(|_| RefError::OutOfRange)?;
        let encoded = slot.checked_add(1).ok_or(RefError::OutOfRange)?;
        Ok(CompressedRef(encoded))
    }

    pub fn decode_offset(self, value: CompressedRef) -> Result<Option<usize>, RefError> {
        let Some(slot) = NonZeroU32::new(value.raw()) else {
            return Ok(None);
        };
        let offset = ((slot.get() - 1) as usize)
            .checked_shl(self.shift as u32)
            .ok_or(RefError::Overflow)?;
        if offset >= self.byte_len {
            return Err(RefError::OutOfRange);
        }
        Ok(Some(offset))
    }

    /// Decode only after validating provenance, generation, lifetime and bounds.
    /// Returns an address as usize, not a Rust reference or exposed FFI pointer.
    pub fn decode_address(self, value: CompressedRef) -> Result<Option<usize>, RefError> {
        self.decode_offset(value)?
            .map(|offset| self.base.checked_add(offset).ok_or(RefError::Overflow))
            .transpose()
    }
}

/// AoSoA index arithmetic; a field's byte address is computed by its
/// independent block stride and element stride, never by an AoS cast.
pub fn aosoa_field_offset(
    row: usize,
    lanes: usize,
    block_stride: usize,
    field_base: usize,
    field_element_stride: usize,
) -> Result<usize, RefError> {
    if lanes == 0 || !lanes.is_power_of_two() || block_stride == 0 {
        return Err(RefError::InvalidLayout);
    }
    let block = row / lanes;
    let lane = row % lanes;
    block
        .checked_mul(block_stride)
        .and_then(|n| n.checked_add(field_base))
        .and_then(|n| {
            lane.checked_mul(field_element_stride)
                .and_then(|v| n.checked_add(v))
        })
        .ok_or(RefError::Overflow)
}

/// ECS generational handle; this is an identity, not a memory address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntityHandle {
    pub slot: u32,
    pub generation: u32,
}

impl EntityHandle {
    pub fn is_current(self, current_generation: u32) -> bool {
        self.generation == current_generation
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_and_first_object() {
        let r = Region::new(0x1000, 4096, 3).unwrap();
        assert_eq!(r.decode_address(CompressedRef::NULL).unwrap(), None);
        let first = r.encode_offset(0).unwrap();
        assert_eq!(first.raw(), 1);
        assert_eq!(r.decode_address(first).unwrap(), Some(0x1000));
    }

    #[test]
    fn round_trip_and_rejections() {
        let r = Region::new(0x1000, 4096, 3).unwrap();
        for offset in (0..4096).step_by(8) {
            assert_eq!(
                r.decode_offset(r.encode_offset(offset).unwrap()).unwrap(),
                Some(offset)
            );
        }
        assert_eq!(r.encode_offset(7), Err(RefError::BadAlignment));
        assert_eq!(r.encode_offset(4096), Err(RefError::OutOfRange));
        assert_eq!(
            r.decode_offset(CompressedRef::from_raw(1000)),
            Err(RefError::OutOfRange)
        );
    }

    #[test]
    fn region_and_overflow() {
        assert_eq!(Region::new(3, 10, 3).unwrap_err(), RefError::BadAlignment);
        assert_eq!(
            Region::new(usize::MAX - 7, 16, 3).unwrap_err(),
            RefError::Overflow
        );
        let r = Region::new(0, usize::MAX, 3).unwrap();
        assert_eq!(
            r.encode_offset((u32::MAX as usize) << 3),
            Err(RefError::OutOfRange)
        );
    }

    #[test]
    fn separate_regions_same_bits_different_addresses() {
        let a = Region::new(0x1000, 4096, 3).unwrap();
        let b = Region::new(0x3000, 4096, 3).unwrap();
        let raw = a.encode_offset(80).unwrap();
        assert_eq!(a.decode_address(raw).unwrap(), Some(0x1050));
        assert_eq!(b.decode_address(raw).unwrap(), Some(0x3050));
        // This does NOT authorize copying raw references between actor domains.
    }

    #[test]
    fn aosoa_arithmetic() {
        assert_eq!(aosoa_field_offset(9, 8, 192, 32, 4).unwrap(), 228);
        assert_eq!(
            aosoa_field_offset(0, 0, 192, 32, 4),
            Err(RefError::InvalidLayout)
        );
    }

    #[test]
    fn stale_entity_handle() {
        let h = EntityHandle {
            slot: 42,
            generation: 8,
        };
        assert!(h.is_current(8));
        assert!(!h.is_current(9));
    }
}
