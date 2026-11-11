//! A deliberately strict envelope for the restricted canonical V0 checker.
//!
//! A producer must independently enumerate every ownership-relevant instruction.
//! This wrapper detects inconsistent, incomplete *submitted* inventories. It is
//! not an independent authentication of the source compiler or its emitted code.

use std::collections::BTreeSet;

use crate::{verify, verify_with_shadow_v0, Block, Diagnostic, Op, Program, Proof, ShadowEventV0};

pub const CONTRACT_VERSION: &str = "atto-borrow-mir-v1";
const MAX_MIR_OPERATIONS: usize = 100_000;
const MAX_MIR_BLOCKS: usize = 4096;

/// Metadata needed for reproducibility, NOT cryptographic attestation.
/// Trusted CI must bind these claims to checked-out bytes independently.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Producer {
    pub compiler_git_sha: String,
    pub source_sha256: String,
    pub source_unit_id: u64,
    pub function_id: u64,
}

/// Every ownership-relevant operation must have a closed effect category.
/// A compiler adapter must reject anything it cannot faithfully represent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Effect {
    Ownership,
    Suspension,
    ActorBoundary,
    Control,
    /// Native FFI, unwind, cancellation, unknown lowering etc. fail closed.
    Unsupported,
}

#[derive(Clone, Debug)]
pub struct Instruction {
    pub instruction_id: u64,
    pub source_node_id: u64,
    pub effect: Effect,
    pub operation: Op,
}

#[derive(Clone, Debug)]
pub struct MirBlock {
    pub id: u32,
    pub instructions: Vec<Instruction>,
    pub successors: Vec<u32>,
}

/// expected_instruction_ids is supplied by an *independent* compiler-side
/// census over validated typed MIR, never by filtering this same event stream.
/// This contract checks internal consistency, not the census's authenticity.
#[derive(Clone, Debug)]
pub struct MirFunction {
    pub version: String,
    pub producer: Producer,
    pub entry: u32,
    pub expected_instruction_ids: Vec<u64>,
    pub blocks: Vec<MirBlock>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MirError {
    Invalid(&'static str),
    Unsafe(Diagnostic),
}

fn is_hex(value: &str, expected_len: usize) -> bool {
    value.len() == expected_len && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn expected_effect(op: &Op) -> Effect {
    match op {
        Op::Suspend => Effect::Suspension,
        Op::ActorHopBorrow { .. }
        | Op::ActorSendPrepare { .. }
        | Op::ActorSendCommit { .. }
        | Op::ActorSendAbort { .. }
        | Op::ActorSendMove { .. }
        | Op::ActorSendCopy { .. }
        | Op::PublishFrozen { .. }
        | Op::PromoteGuarded { .. } => Effect::ActorBoundary,
        Op::Return => Effect::Control,
        _ => Effect::Ownership,
    }
}

/// Validate the envelope, then invoke the original restricted V0 semantics.
///
/// Success is a MODEL result, not compiled-guest execution or verified
/// source-to-MIR-to-machine-code completeness.
pub fn verify_mir(function: &MirFunction) -> Result<Proof, MirError> {
    return verify(&validate_and_lower(function)?).map_err(MirError::Unsafe);
}

/// Validate the complete MIR inventory before observing the verified CFG.
/// Invalid inventories and unsafe graphs publish no events. A sink failure
/// prevents a proof, but already performed observer side effects cannot roll back.
pub fn verify_mir_with_shadow_v0(
    function: &MirFunction,
    sink: &mut dyn FnMut(ShadowEventV0) -> Result<(), Diagnostic>,
) -> Result<Proof, MirError> {
    return verify_with_shadow_v0(&validate_and_lower(function)?, sink).map_err(MirError::Unsafe);
}

fn validate_and_lower(function: &MirFunction) -> Result<Program, MirError> {
    if function.version != CONTRACT_VERSION {
        return Err(MirError::Invalid("E_MIR_VERSION"));
    }
    // Bound the input before allocating the internal CFG copy. The underlying
    // V0 verifier checks the same block limit, but that happens after cloning.
    if function.blocks.is_empty() || function.blocks.len() > MAX_MIR_BLOCKS {
        return Err(MirError::Invalid("E_MIR_CFG_BUDGET"));
    }
    // Preflight every vector length before cloning any caller-owned CFG data.
    let mut edges = 0usize;
    let mut operations = 0usize;
    for block in &function.blocks {
        edges = edges.saturating_add(block.successors.len());
        operations = operations.saturating_add(block.instructions.len());
        if block.instructions.len() > crate::MAX_OPERATIONS_PER_BLOCK || edges > crate::MAX_EDGES {
            return Err(MirError::Invalid("E_MIR_CFG_BUDGET"));
        }
        if operations > MAX_MIR_OPERATIONS {
            return Err(MirError::Invalid("E_MIR_INVENTORY_BUDGET"));
        }
    }
    let producer = &function.producer;
    if !is_hex(&producer.compiler_git_sha, 40)
        || !is_hex(&producer.source_sha256, 64)
        || producer.source_unit_id == 0
        || producer.function_id == 0
    {
        return Err(MirError::Invalid("E_MIR_PRODUCER"));
    }
    if function.expected_instruction_ids.is_empty()
        || function.expected_instruction_ids.len() > MAX_MIR_OPERATIONS
    {
        return Err(MirError::Invalid("E_MIR_INVENTORY_BUDGET"));
    }
    let mut expected = BTreeSet::new();
    for &id in &function.expected_instruction_ids {
        if id == 0 || !expected.insert(id) {
            return Err(MirError::Invalid("E_MIR_DUPLICATE_EXPECTED_ID"));
        }
    }
    let mut observed = BTreeSet::new();
    let mut total = 0usize;
    let mut blocks = Vec::with_capacity(function.blocks.len());
    for block in &function.blocks {
        total = total.saturating_add(block.instructions.len());
        if total > MAX_MIR_OPERATIONS {
            return Err(MirError::Invalid("E_MIR_INVENTORY_BUDGET"));
        }
        let mut ops = Vec::with_capacity(block.instructions.len());
        for instruction in &block.instructions {
            if instruction.instruction_id == 0
                || instruction.source_node_id == 0
                || !observed.insert(instruction.instruction_id)
            {
                return Err(MirError::Invalid("E_MIR_DUPLICATE_OR_MISSING_ID"));
            }
            if instruction.effect == Effect::Unsupported {
                return Err(MirError::Invalid("E_MIR_UNSUPPORTED_EFFECT"));
            }
            if instruction.effect != expected_effect(&instruction.operation) {
                return Err(MirError::Invalid("E_MIR_EFFECT_MISMATCH"));
            }
            ops.push(instruction.operation.clone());
        }
        blocks.push(Block {
            id: block.id,
            operations: ops,
            successors: block.successors.clone(),
        });
    }
    if expected != observed {
        return Err(MirError::Invalid("E_MIR_INCOMPLETE_INVENTORY"));
    }
    return Ok(Program {
        entry: function.entry,
        blocks,
    });
}
