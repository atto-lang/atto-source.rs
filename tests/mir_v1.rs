use atto_borrow_checker::mir_v1::{
    verify_mir, Effect, Instruction, MirBlock, MirError, MirFunction, Producer, CONTRACT_VERSION,
};
use atto_borrow_checker::{Op, Traits};

fn instruction(id: u64, effect: Effect, operation: Op) -> Instruction {
    Instruction {
        instruction_id: id,
        source_node_id: id + 100,
        effect,
        operation,
    }
}

fn good() -> MirFunction {
    MirFunction {
        version: CONTRACT_VERSION.into(),
        producer: Producer {
            compiler_git_sha: "a".repeat(40),
            source_sha256: "b".repeat(64),
            source_unit_id: 1,
            function_id: 2,
        },
        entry: 0,
        expected_instruction_ids: vec![1, 2],
        blocks: vec![MirBlock {
            id: 0,
            instructions: vec![
                instruction(
                    1,
                    Effect::Ownership,
                    Op::Init {
                        owner: 1,
                        traits: Traits {
                            is_copy: false,
                            is_send: false,
                        },
                    },
                ),
                instruction(2, Effect::Control, Op::Return),
            ],
            successors: vec![],
        }],
    }
}

fn reject_invalid(function: &MirFunction, expected: &'static str) {
    assert_eq!(verify_mir(function), Err(MirError::Invalid(expected)));
}

#[test]
fn accepted_restricted_program_is_model_only() {
    let proof = verify_mir(&good()).expect("restricted ownership model should accept");
    assert_eq!(proof.cleanup_by_exit[&0], vec![1]);
}

#[test]
fn rejects_version_drift() {
    let mut mir = good();
    mir.version = "atto-borrow-mir-v0".into();
    reject_invalid(&mir, "E_MIR_VERSION");
}

#[test]
fn rejects_missing_compiler_and_source_provenance() {
    let mut mir = good();
    mir.producer.compiler_git_sha = "fabricated".into();
    reject_invalid(&mir, "E_MIR_PRODUCER");
    let mut mir = good();
    mir.producer.source_sha256 = "".into();
    reject_invalid(&mir, "E_MIR_PRODUCER");
    let mut mir = good();
    mir.producer.function_id = 0;
    reject_invalid(&mir, "E_MIR_PRODUCER");
}

#[test]
fn rejects_unmodeled_foreign_or_unwind_effects() {
    let mut mir = good();
    mir.blocks[0].instructions[0].effect = Effect::Unsupported;
    reject_invalid(&mir, "E_MIR_UNSUPPORTED_EFFECT");
}

#[test]
fn rejects_suspension_disguised_as_ordinary_ownership() {
    let mut mir = good();
    mir.blocks[0].instructions[0].operation = Op::Suspend;
    reject_invalid(&mir, "E_MIR_EFFECT_MISMATCH");
}

#[test]
fn rejects_wrong_control_effect() {
    let mut mir = good();
    mir.blocks[0].instructions[1].effect = Effect::Ownership;
    reject_invalid(&mir, "E_MIR_EFFECT_MISMATCH");
}

#[test]
fn rejects_omitted_typed_mir_operation() {
    let mut mir = good();
    mir.expected_instruction_ids.push(3);
    reject_invalid(&mir, "E_MIR_INCOMPLETE_INVENTORY");
}

#[test]
fn rejects_extra_unaccounted_operation() {
    let mut mir = good();
    mir.blocks[0]
        .instructions
        .insert(1, instruction(3, Effect::Ownership, Op::Read { owner: 1 }));
    reject_invalid(&mir, "E_MIR_INCOMPLETE_INVENTORY");
}

#[test]
fn rejects_duplicate_expected_identity() {
    let mut mir = good();
    mir.expected_instruction_ids.push(2);
    reject_invalid(&mir, "E_MIR_DUPLICATE_EXPECTED_ID");
}

#[test]
fn rejects_duplicate_emitted_identity() {
    let mut mir = good();
    mir.blocks[0].instructions[1].instruction_id = 1;
    reject_invalid(&mir, "E_MIR_DUPLICATE_OR_MISSING_ID");
}

#[test]
fn rejects_unknown_source_node() {
    let mut mir = good();
    mir.blocks[0].instructions[0].source_node_id = 0;
    reject_invalid(&mir, "E_MIR_DUPLICATE_OR_MISSING_ID");
}

#[test]
fn propagates_semantic_use_after_move_error() {
    let mut mir = good();
    mir.blocks[0].instructions.insert(
        1,
        instruction(3, Effect::Ownership, Op::Move { from: 1, into: 2 }),
    );
    mir.blocks[0]
        .instructions
        .insert(2, instruction(4, Effect::Ownership, Op::Read { owner: 1 }));
    mir.expected_instruction_ids.extend([3, 4]);
    let error = verify_mir(&mir).unwrap_err();
    match error {
        MirError::Unsafe(diagnostic) => assert_eq!(diagnostic.code, "E_USE_AFTER_MOVE_OR_DROP"),
        _ => panic!("expected semantic borrow failure"),
    }
}

#[test]
fn rejects_forged_implicit_exit() {
    let mut mir = good();
    mir.blocks[0].instructions.pop();
    mir.expected_instruction_ids.pop();
    let error = verify_mir(&mir).unwrap_err();
    match error {
        MirError::Unsafe(diagnostic) => assert_eq!(diagnostic.code, "E_IMPLICIT_EXIT"),
        _ => panic!("expected canonical CFG rejection"),
    }
}

#[test]
fn rejects_cfg_cycles_until_sound_loop_proof_is_available() {
    let mut mir = good();
    mir.blocks[0].instructions.pop();
    mir.expected_instruction_ids.pop();
    mir.blocks[0].successors.push(0);
    let error = verify_mir(&mir).unwrap_err();
    match error {
        MirError::Unsafe(diagnostic) => assert_eq!(diagnostic.code, "E_CFG_CYCLE_UNPROVEN"),
        _ => panic!("expected canonical cycle rejection"),
    }
}

#[test]
fn accepts_explicit_loan_release_before_suspension() {
    let mut mir = good();
    mir.blocks[0].instructions.insert(
        1,
        instruction(
            3,
            Effect::Ownership,
            Op::Borrow {
                owner: 1,
                loan: 10,
                kind: atto_borrow_checker::BorrowKind::Shared,
            },
        ),
    );
    mir.blocks[0].instructions.insert(
        2,
        instruction(4, Effect::Ownership, Op::EndBorrow { loan: 10 }),
    );
    mir.blocks[0]
        .instructions
        .insert(3, instruction(5, Effect::Suspension, Op::Suspend));
    mir.expected_instruction_ids.extend([3, 4, 5]);
    verify_mir(&mir).expect("released shared loan before suspension");
}

#[test]
fn rejects_oversized_cfg_before_allocating_internal_copy() {
    let mut mir = good();
    mir.blocks = (0..4097)
        .map(|id| MirBlock {
            id,
            instructions: Vec::new(),
            successors: Vec::new(),
        })
        .collect();
    reject_invalid(&mir, "E_MIR_CFG_BUDGET");
}

#[test]
fn rejects_empty_cfg_even_with_inventory_claims() {
    let mut mir = good();
    mir.blocks.clear();
    reject_invalid(&mir, "E_MIR_CFG_BUDGET");
}

#[test]
fn reports_duplicate_owner_definition_from_canonical_engine() {
    let mut mir = good();
    mir.blocks[0].instructions.insert(
        1,
        instruction(
            3,
            Effect::Ownership,
            Op::Init {
                owner: 1,
                traits: Traits {
                    is_copy: false,
                    is_send: false,
                },
            },
        ),
    );
    mir.expected_instruction_ids.push(3);
    match verify_mir(&mir).unwrap_err() {
        MirError::Unsafe(diagnostic) => assert_eq!(diagnostic.code, "E_OWNER_DEF_REUSED"),
        _ => panic!("expected global identity collision"),
    }
}

#[test]
fn verified_mir_shadow_matches_plain_cleanup_and_publishes_complete_trace() {
    let mir = good();
    let mut events = Vec::new();
    let result = atto_borrow_checker::mir_v1::verify_mir_with_shadow_v0(&mir, &mut |event| {
        events.push(event);
        Ok(())
    });
    assert_eq!(result, verify_mir(&mir));
    assert_eq!(
        events.iter().map(|event| event.kind).collect::<Vec<_>>(),
        vec!["INIT", "RETURN"]
    );
}

#[test]
fn invalid_inventory_never_publishes_shadow_events() {
    let mut mir = good();
    mir.expected_instruction_ids.push(3);
    let mut events = Vec::new();
    let result = atto_borrow_checker::mir_v1::verify_mir_with_shadow_v0(&mir, &mut |event| {
        events.push(event);
        Ok(())
    });
    assert_eq!(result, Err(MirError::Invalid("E_MIR_INCOMPLETE_INVENTORY")));
    assert!(events.is_empty());
}

#[test]
fn unsafe_mir_never_publishes_shadow_events() {
    let mut mir = good();
    mir.expected_instruction_ids.extend([3, 4]);
    mir.blocks[0]
        .instructions
        .insert(1, instruction(3, Effect::Ownership, Op::Drop { owner: 1 }));
    mir.blocks[0]
        .instructions
        .insert(2, instruction(4, Effect::Ownership, Op::Read { owner: 1 }));
    let mut events = Vec::new();
    let result = atto_borrow_checker::mir_v1::verify_mir_with_shadow_v0(&mir, &mut |event| {
        events.push(event);
        Ok(())
    });
    assert!(
        matches!(result, Err(MirError::Unsafe(error)) if error.code == "E_USE_AFTER_MOVE_OR_DROP")
    );
    assert!(events.is_empty());
}

#[test]
fn mir_observer_failure_does_not_return_proof() {
    let result = atto_borrow_checker::mir_v1::verify_mir_with_shadow_v0(&good(), &mut |_| {
        Err(atto_borrow_checker::Diagnostic {
            code: "E_OBSERVER",
            block: 0,
            operation: 0,
        })
    });
    assert!(matches!(result, Err(MirError::Unsafe(error)) if error.code == "E_OBSERVER"));
}

#[test]
fn oversized_successor_vector_is_rejected_before_lowering() {
    let mut mir = good();
    mir.blocks[0].successors = vec![0; 16_385];
    reject_invalid(&mir, "E_MIR_CFG_BUDGET");
}

#[test]
fn cumulative_edge_budget_is_checked_before_lowering() {
    let mut mir = good();
    mir.blocks[0].successors = vec![0; 8192];
    let mut second = mir.blocks[0].clone();
    second.id = 1;
    second.successors.push(0);
    mir.blocks.push(second);
    reject_invalid(&mir, "E_MIR_CFG_BUDGET");
}

#[test]
fn oversized_block_operation_vector_is_rejected_before_lowering() {
    let mut mir = good();
    mir.blocks[0].instructions = vec![instruction(1, Effect::Control, Op::Return); 1025];
    reject_invalid(&mir, "E_MIR_CFG_BUDGET");
}
