//! Compiler-controlled proof gate for the browser actor DATA ABI.
//!
//! This is intentionally narrow: the current guest actor_send(ref, i64) has
//! no Result/ownership-return path, brokered wire codec or deep-freeze ABI.
//! Therefore only the two admitted primitive payload types can be copied.
//! This is *not* a proof of runtime TenantVM confinement or host principal
//! authentication; those require an independent real guest/runtime gate.
#![forbid(unsafe_code)]
#![cfg_attr(not(test), allow(dead_code))]

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PayloadShape {
    Int,
    Bool,
    NonScalar,
    ActorCapability,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TransferIntent {
    GuestScalarCopy,
    ExclusiveTake,
    ImmutableShare,
    CrossTenantWire,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProofError {
    NonSendable,
    NoTransactionalAdmission,
    NoFreezeProof,
    NoWireCodec,
}

// Not source-constructible. Only a checked compiler type can call the gate,
// and guest code cannot use type declarations to mint a capability.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ScalarCopyProof {
    kind: PayloadShape,
}

impl ScalarCopyProof {
    pub(crate) fn type_tag(self) -> u8 {
        match self.kind {
            PayloadShape::Int => 1,
            PayloadShape::Bool => 2,
            _ => unreachable!("constructor only returns typed scalars"),
        }
    }
}

pub(crate) fn prove_payload(shape: PayloadShape, intent: TransferIntent)
    -> Result<ScalarCopyProof, ProofError> {
    // A caller may not silently downgrade take!/share!/wire to scalar-copy
    // even for a primitive. The intent must match the actual guest ABI.
    match intent {
        TransferIntent::ExclusiveTake => return Err(ProofError::NoTransactionalAdmission),
        TransferIntent::ImmutableShare => return Err(ProofError::NoFreezeProof),
        TransferIntent::CrossTenantWire => return Err(ProofError::NoWireCodec),
        TransferIntent::GuestScalarCopy => {}
    }
    match shape {
        PayloadShape::Int | PayloadShape::Bool => Ok(ScalarCopyProof { kind: shape }),
        PayloadShape::NonScalar | PayloadShape::ActorCapability => Err(ProofError::NonSendable),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn int_and_bool_receive_compiler_owned_scalar_copy_proof() {
        assert_eq!(prove_payload(PayloadShape::Int, TransferIntent::GuestScalarCopy)
            .unwrap().type_tag(), 1);
        assert_eq!(prove_payload(PayloadShape::Bool, TransferIntent::GuestScalarCopy)
            .unwrap().type_tag(), 2);
    }
    #[test] fn nominal_classes_and_actor_refs_cannot_claim_scalar_proof() {
        for shape in [PayloadShape::NonScalar, PayloadShape::ActorCapability] {
            assert_eq!(prove_payload(shape, TransferIntent::GuestScalarCopy),
                Err(ProofError::NonSendable));
        }
    }
    #[test] fn no_implicit_move_or_returnless_take() {
        for shape in [PayloadShape::Int, PayloadShape::Bool, PayloadShape::NonScalar,
                      PayloadShape::ActorCapability] {
            assert_eq!(prove_payload(shape, TransferIntent::ExclusiveTake),
                Err(ProofError::NoTransactionalAdmission));
        }
    }
    #[test] fn no_implicit_freeze_or_mutable_alias_sharing() {
        for shape in [PayloadShape::Int, PayloadShape::Bool, PayloadShape::NonScalar,
                      PayloadShape::ActorCapability] {
            assert_eq!(prove_payload(shape, TransferIntent::ImmutableShare),
                Err(ProofError::NoFreezeProof));
        }
    }
    #[test] fn scalar_layout_does_not_grant_cross_tenant_wire() {
        for shape in [PayloadShape::Int, PayloadShape::Bool, PayloadShape::NonScalar,
                      PayloadShape::ActorCapability] {
            assert_eq!(prove_payload(shape, TransferIntent::CrossTenantWire),
                Err(ProofError::NoWireCodec));
        }
    }
}
