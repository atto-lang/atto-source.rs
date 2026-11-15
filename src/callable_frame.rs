//! Proposed owned, scalar capture environment and generational callable-handle
//! reference model. This is NOT guest-Wasm closure lowering. The ordinary
//! compiler still rejects escaped callable values and call_indirect.
//!
//! The handle is a private per-instance (generation, one-based slot) pair.
//! An owner ID is checked against the arena on every operation; a raw integer
//! cannot be turned into an AttoLang callable by source code or host messages.
//! A runtime implementing this contract must restrict access to checked
//! handles and must not deserialize them across actors or tenants.

use crate::ast::Type;
use crate::check::callable_ir::CallableSignature;

pub const MAX_CALLABLE_SLOTS: usize = 1024;
pub const MAX_CAPTURE_WORDS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallableFault {
    InvalidOwner,
    InvalidHandle,
    StaleHandle,
    SignatureMismatch,
    Capacity,
    InvalidTarget,
    InvalidCapture,
}

/// Only independently copyable capture values are accepted by this model.
/// Borrows, objects, actor state, and string slices are not implicitly owned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScalarCapture {
    Int(i64),
    Bool(bool),
    FloatBits(u64),
}
impl Default for ScalarCapture {
    fn default() -> Self { Self::Int(0) }
}
impl ScalarCapture {
    pub fn ty(self) -> Type {
        match self {
            Self::Int(_) => Type::Int,
            Self::Bool(_) => Type::Bool,
            Self::FloatBits(_) => Type::Float,
        }
    }
}

/// Opaque compiler/runtime capability, NOT a user-constructible integer cast.
#[derive(Debug, PartialEq, Eq)]
pub struct CallableHandle(u64);
impl CallableHandle {
    pub fn bits(&self) -> u64 { self.0 }
}

/// Borrowed read of a validated live environment. The borrow cannot outlive
/// the arena or coexist with a mutable close/recycle operation.
pub struct CallableView<'a> {
    pub function_index: u32,
    pub captures: &'a [ScalarCapture],
}

#[derive(Debug)]
struct Slot {
    generation: u32,
    owner: u64,
    signature: Option<CallableSignature>,
    function_index: u32,
    captures: [ScalarCapture; MAX_CAPTURE_WORDS],
    capture_count: usize,
    live: bool,
    retired: bool,
}
impl Slot {
    fn vacant() -> Self {
        Self {
            generation: 1, owner: 0, signature: None, function_index: 0,
            captures: [ScalarCapture::default(); MAX_CAPTURE_WORDS],
            capture_count: 0, live: false, retired: false,
        }
    }
}

/// One arena belongs to exactly one guest instance/actor execution domain.
/// Callers must supply that domain's trusted owner identity on access.
#[derive(Debug)]
pub struct CallableArena {
    owner: u64,
    slots: Vec<Slot>,
    free: Vec<u32>,
}
impl CallableArena {
    pub fn new(owner: u64) -> Result<Self, CallableFault> {
        if owner == 0 { return Err(CallableFault::InvalidOwner); }
        Ok(Self { owner, slots: Vec::new(), free: Vec::new() })
    }

    /// Capture layout is checked separately from a callable's argument and
    /// result signature. The actual typed compiler must validate each binding
    /// at its definition site before invoking this arena.
    pub fn allocate(
        &mut self, caller_owner: u64, signature: CallableSignature,
        function_index: u32, captures: &[ScalarCapture],
    ) -> Result<CallableHandle, CallableFault> {
        self.check_owner(caller_owner)?;
        if function_index >= crate::parser::MAX_FUNCS as u32 {
            return Err(CallableFault::InvalidTarget);
        }
        if captures.len() > MAX_CAPTURE_WORDS {
            return Err(CallableFault::InvalidCapture);
        }
        if signature.params.len() > crate::parser::MAX_PARAMS
            || signature.params.iter().any(|t| !matches!(t,
                Type::Int | Type::Bool | Type::Float | Type::Str))
            || !matches!(signature.result,
                Type::Int | Type::Bool | Type::Float | Type::Str | Type::Void) {
            return Err(CallableFault::SignatureMismatch);
        }
        // One-based slot number keeps 0 invalid even in generation 1.
        let slot_idx = if let Some(index) = self.free.pop() {
            (index - 1) as usize
        } else {
            if self.slots.len() >= MAX_CALLABLE_SLOTS {
                return Err(CallableFault::Capacity);
            }
            self.slots.push(Slot::vacant());
            self.slots.len() - 1
        };
        let slot = &mut self.slots[slot_idx];
        // Never recycle a retired or live slot even if a free-list bug occurs.
        if slot.live || slot.retired || slot.generation == 0 {
            return Err(CallableFault::InvalidHandle);
        }
        slot.owner = self.owner;
        slot.signature = Some(signature);
        slot.function_index = function_index;
        slot.captures[..captures.len()].copy_from_slice(captures);
        slot.capture_count = captures.len();
        slot.live = true;
        let number = slot_idx as u64 + 1;
        Ok(CallableHandle(((slot.generation as u64) << 32) | number))
    }

    fn check_owner(&self, caller_owner: u64) -> Result<(), CallableFault> {
        if caller_owner == 0 || caller_owner != self.owner {
            Err(CallableFault::InvalidOwner)
        } else {
            Ok(())
        }
    }

    fn index(&self, bits: u64, caller_owner: u64) -> Result<usize, CallableFault> {
        self.check_owner(caller_owner)?;
        let number = bits as u32;
        let generation = (bits >> 32) as u32;
        if number == 0 || generation == 0 || number as usize > self.slots.len() {
            return Err(CallableFault::InvalidHandle);
        }
        let index = number as usize - 1;
        let slot = &self.slots[index];
        if slot.generation != generation || !slot.live || slot.retired {
            return Err(CallableFault::StaleHandle);
        }
        if slot.owner != caller_owner { return Err(CallableFault::InvalidOwner); }
        Ok(index)
    }

    /// Guest-private ingress validation. No source-language numeric conversion
    /// or host serialization is permitted to call this as an authorization step.
    pub fn validate_bits<'a>(
        &'a self, bits: u64, caller_owner: u64,
        expected: &CallableSignature,
    ) -> Result<CallableView<'a>, CallableFault> {
        let slot = &self.slots[self.index(bits, caller_owner)?];
        if slot.signature.as_ref() != Some(expected) {
            return Err(CallableFault::SignatureMismatch);
        }
        Ok(CallableView {
            function_index: slot.function_index,
            captures: &slot.captures[..slot.capture_count],
        })
    }

    /// Close once. Scrub all captures before a slot can be recycled. A
    /// generation reaching u32::MAX is retired rather than wrapping to 1.
    pub fn close(&mut self, bits: u64, caller_owner: u64) -> Result<(), CallableFault> {
        let index = self.index(bits, caller_owner)?;
        let slot = &mut self.slots[index];
        slot.captures.fill(ScalarCapture::default());
        slot.capture_count = 0;
        slot.signature = None;
        slot.function_index = 0;
        slot.owner = 0;
        slot.live = false;
        if let Some(next) = slot.generation.checked_add(1) {
            slot.generation = next;
            self.free.push((index + 1) as u32);
        } else {
            slot.retired = true;
        }
        Ok(())
    }

    pub fn live_count(&self) -> usize {
        self.slots.iter().filter(|s| s.live).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diag::Pos;
    fn sig() -> CallableSignature {
        CallableSignature::scalar(vec![Type::Int], Type::Int,
            Pos { line: 1, col: 1 }).unwrap()
    }
    fn invoke_add(arena: &CallableArena, handle: u64, owner: u64, n: i64) -> i64 {
        let checked = arena.validate_bits(handle, owner, &sig()).unwrap();
        assert_eq!(checked.function_index, 0);
        let ScalarCapture::Int(offset) = checked.captures[0] else {
            panic!("expected immutable integer capture");
        };
        n.checked_add(offset).unwrap()
    }
    fn make_adder(arena: &mut CallableArena, offset: i64) -> CallableHandle {
        arena.allocate(7, sig(), 0, &[ScalarCapture::Int(offset)]).unwrap()
    }
    #[test]
    fn escaping_factory_preserves_its_owned_capture_after_return() {
        let mut arena = CallableArena::new(7).unwrap();
        let add_one = make_adder(&mut arena, 1);
        assert_eq!(invoke_add(&arena, add_one.bits(), 7, 41), 42);
        arena.close(add_one.bits(), 7).unwrap();
        assert_eq!(arena.live_count(), 0);
    }
    #[test]
    fn independent_instances_and_higher_order_reference_semantics() {
        let mut arena = CallableArena::new(7).unwrap();
        let one = make_adder(&mut arena, 1);
        let two = make_adder(&mut arena, 2);
        let apply_twice = |h| invoke_add(&arena, h, 7, invoke_add(&arena, h, 7, 40));
        assert_eq!(apply_twice(one.bits()), 42);
        assert_eq!(apply_twice(two.bits()), 44);
        arena.close(one.bits(), 7).unwrap();
        assert_eq!(invoke_add(&arena, two.bits(), 7, 40), 42);
    }
    #[test]
    fn rejects_stale_forged_zero_and_out_of_range_handles() {
        let mut arena = CallableArena::new(7).unwrap();
        let first = make_adder(&mut arena, 3);
        let old = first.bits();
        assert_eq!(arena.validate_bits(0, 7, &sig()).err(),
            Some(CallableFault::InvalidHandle));
        assert_eq!(arena.validate_bits((1u64 << 32) | 200, 7, &sig()).err(),
            Some(CallableFault::InvalidHandle));
        arena.close(old, 7).unwrap();
        assert_eq!(arena.validate_bits(old, 7, &sig()).err(),
            Some(CallableFault::StaleHandle));
        assert_eq!(arena.close(old, 7), Err(CallableFault::StaleHandle));
        let fresh = make_adder(&mut arena, 4);
        assert_ne!(old, fresh.bits());
        assert_eq!(invoke_add(&arena, fresh.bits(), 7, 38), 42);
        assert_eq!(arena.validate_bits(old, 7, &sig()).err(),
            Some(CallableFault::StaleHandle));
    }
    #[test]
    fn rejects_cross_actor_and_wrong_signature_dispatch() {
        let mut arena = CallableArena::new(7).unwrap();
        let h = make_adder(&mut arena, 1);
        assert_eq!(arena.validate_bits(h.bits(), 8, &sig()).err(),
            Some(CallableFault::InvalidOwner));
        assert_eq!(arena.close(h.bits(), 8), Err(CallableFault::InvalidOwner));
        let different = CallableSignature::scalar(vec![Type::Bool], Type::Int,
            Pos { line: 1, col: 1 }).unwrap();
        assert_eq!(arena.validate_bits(h.bits(), 7, &different).err(),
            Some(CallableFault::SignatureMismatch));
    }
    #[test]
    fn guards_invalid_ownership_target_and_capture_limits() {
        assert!(matches!(CallableArena::new(0), Err(CallableFault::InvalidOwner)));
        let mut arena = CallableArena::new(7).unwrap();
        assert_eq!(arena.allocate(8, sig(), 0, &[]).unwrap_err(),
            CallableFault::InvalidOwner);
        assert_eq!(arena.allocate(7, sig(), crate::parser::MAX_FUNCS as u32, &[]).unwrap_err(),
            CallableFault::InvalidTarget);
        assert_eq!(arena.allocate(7, sig(), 0,
            &vec![ScalarCapture::Int(1); MAX_CAPTURE_WORDS + 1]).unwrap_err(),
            CallableFault::InvalidCapture);
        assert_eq!(arena.live_count(), 0);
    }
    #[test]
    fn close_scrubs_captures_and_never_recycles_exhausted_generations() {
        let mut arena = CallableArena::new(7).unwrap();
        let h = make_adder(&mut arena, 15);
        let bits = h.bits();
        arena.close(bits, 7).unwrap();
        assert!(arena.slots[0].captures.iter().all(|c| *c == ScalarCapture::Int(0)));
        // Reach exhaustion directly rather than performing four billion cycles.
        arena.free.clear();
        arena.slots[0].generation = u32::MAX;
        arena.slots[0].live = true;
        arena.slots[0].owner = 7;
        arena.slots[0].signature = Some(sig());
        let last = ((u32::MAX as u64) << 32) | 1;
        arena.close(last, 7).unwrap();
        assert!(arena.slots[0].retired);
        assert!(arena.free.is_empty());
        let next = make_adder(&mut arena, 1);
        assert_eq!(next.bits() as u32, 2);
    }
}
