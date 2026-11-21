//! Four-byte CONTROL-plane slot/generation storage ABI, separate from
//! actor-local offset8_plus_one and the .NET generation16_slot16 actor ABI.
//! A decoded word proves ONLY a well-formed bit pattern, not live object,
//! issuing control domain, actor authority, incarnation, type or permissions.
//! The trusted runtime must validate those out-of-band before dereference.

pub const CONTROL_REF32_STORAGE_ABI_V1: &str = "control_generation12_slot20_v1";
pub const CONTROL_SLOT_BITS: u32 = 20;
pub const CONTROL_GENERATION_BITS: u32 = 12;
pub const CONTROL_SLOT_MASK: u32 = (1u32 << CONTROL_SLOT_BITS) - 1;
pub const CONTROL_MAX_GENERATION: u32 = (1u32 << CONTROL_GENERATION_BITS) - 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlRef32Error {
    InvalidBits,
    IncompatibleAbi,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlRef32Fields {
    pub slot_index: u32,
    pub generation: u32,
}

pub fn require_control_ref32_abi_v1(requested: &str) -> Result<(), ControlRef32Error> {
    if requested == CONTROL_REF32_STORAGE_ABI_V1 {
        Ok(())
    } else {
        Err(ControlRef32Error::IncompatibleAbi)
    }
}

/// Mint only an internal storage word. Caller must authenticate its issuer.
pub fn encode_control_ref32_v1(
    slot_index: u32,
    generation: u32,
) -> Result<u32, ControlRef32Error> {
    if slot_index >= CONTROL_SLOT_MASK || generation == 0 || generation > CONTROL_MAX_GENERATION {
        return Err(ControlRef32Error::InvalidBits);
    }
    Ok((generation << CONTROL_SLOT_BITS) | (slot_index + 1))
}

/// Returns None for null and rejects illegal slot/generation-zero words.
pub fn decode_control_ref32_v1(word: u32) -> Result<Option<ControlRef32Fields>, ControlRef32Error> {
    if word == 0 {
        return Ok(None);
    }
    let low = word & CONTROL_SLOT_MASK;
    let generation = word >> CONTROL_SLOT_BITS;
    if low == 0 || generation == 0 {
        return Err(ControlRef32Error::InvalidBits);
    }
    Ok(Some(ControlRef32Fields { slot_index: low - 1, generation }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_abi_is_not_actor_abi() {
        assert_eq!(CONTROL_SLOT_BITS + CONTROL_GENERATION_BITS, 32);
        assert_eq!(CONTROL_SLOT_MASK, 1_048_575);
        assert_eq!(CONTROL_MAX_GENERATION, 4095);
        assert_eq!(require_control_ref32_abi_v1(CONTROL_REF32_STORAGE_ABI_V1), Ok(()));
        for invalid in ["control_generation20_slot12_v1", "generation16_slot16",
            "generation20_slot12", "offset8_plus_one", ""] {
            assert_eq!(require_control_ref32_abi_v1(invalid), Err(ControlRef32Error::IncompatibleAbi));
        }
    }

    #[test]
    fn unsigned_high_bit_and_max_word_roundtrip() {
        assert_eq!(decode_control_ref32_v1(0), Ok(None));
        for (slot_index, generation, expected) in [
            (0, 1, 0x0010_0001),
            (0, 2048, 0x8000_0001),
            (CONTROL_SLOT_MASK - 1, 4095, u32::MAX),
        ] {
            assert_eq!(encode_control_ref32_v1(slot_index, generation), Ok(expected));
            assert_eq!(decode_control_ref32_v1(expected), Ok(Some(ControlRef32Fields {
                slot_index, generation,
            })));
        }
    }

    #[test]
    fn bad_slot_generation_or_zero_fields_fail_closed() {
        for (slot, generation) in [(CONTROL_SLOT_MASK, 1), (u32::MAX, 1),
            (0, 0), (0, 4096), (0, u32::MAX)] {
            assert_eq!(encode_control_ref32_v1(slot,generation), Err(ControlRef32Error::InvalidBits));
        }
        for word in [1u32, 0x0010_0000, 0x8000_0000] {
            assert_eq!(decode_control_ref32_v1(word), Err(ControlRef32Error::InvalidBits));
        }
    }
}
