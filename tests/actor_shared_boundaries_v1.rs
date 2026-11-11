use atto_borrow_checker::mir_v1::{
    verify_mir, Effect, Instruction, MirBlock, MirError, MirFunction, Producer, CONTRACT_VERSION,
};
use atto_borrow_checker::{verify, verify_with_shadow_v0, Block, BorrowKind, Op, Program, Traits};

fn init(owner: u32, copy: bool, send: bool) -> Op {
    Op::Init {
        owner,
        traits: Traits {
            is_copy: copy,
            is_send: send,
        },
    }
}

fn run(operations: Vec<Op>) -> Result<atto_borrow_checker::Proof, atto_borrow_checker::Diagnostic> {
    verify(&Program {
        entry: 0,
        blocks: vec![Block {
            id: 0,
            operations,
            successors: vec![],
        }],
    })
}

fn code(operations: Vec<Op>) -> &'static str {
    run(operations).unwrap_err().code
}

fn read(owner: u32) -> Op {
    Op::Read { owner }
}

fn ret() -> Op {
    Op::Return
}

#[test]
fn owned_actor_send_moves_unique_owner_out_of_sender_cleanup() {
    let proof = run(vec![
        init(1, false, true),
        Op::ActorSendMove { owner: 1 },
        ret(),
    ])
    .expect("Send owner must transfer without a second owner");
    assert!(proof.cleanup_by_exit[&0].is_empty());
    assert_eq!(
        code(vec![
            init(1, false, true),
            Op::ActorSendMove { owner: 1 },
            read(1),
            ret(),
        ]),
        "E_USE_AFTER_MOVE_OR_DROP"
    );
}

#[test]
fn non_send_and_live_loan_cannot_cross_owned_actor_boundary() {
    assert_eq!(
        code(vec![
            init(1, false, false),
            Op::ActorSendMove { owner: 1 },
            ret()
        ]),
        "E_ACTOR_SEND_PROOF_REQUIRED"
    );
    assert_eq!(
        code(vec![
            init(1, false, true),
            Op::Borrow {
                owner: 1,
                loan: 7,
                kind: BorrowKind::Shared,
            },
            Op::ActorSendMove { owner: 1 },
            ret(),
        ]),
        "E_BORROW_CONFLICT"
    );
}

#[test]
fn copied_actor_mail_requires_both_copy_and_send() {
    assert_eq!(
        code(vec![
            init(1, false, true),
            Op::ActorSendCopy { owner: 1 },
            ret()
        ]),
        "E_ACTOR_COPY_SEND_PROOF_REQUIRED"
    );
    assert_eq!(
        code(vec![
            init(1, true, false),
            Op::ActorSendCopy { owner: 1 },
            ret()
        ]),
        "E_ACTOR_COPY_SEND_PROOF_REQUIRED"
    );
    let proof = run(vec![
        init(1, true, true),
        Op::ActorSendCopy { owner: 1 },
        read(1),
        ret(),
    ])
    .expect("Copy + Send sends an independent value; sender still owns its value");
    assert_eq!(proof.cleanup_by_exit[&0], vec![1]);
}

#[test]
fn copied_actor_mail_conflicts_with_exclusive_loan() {
    assert_eq!(
        code(vec![
            init(1, true, true),
            Op::Borrow {
                owner: 1,
                loan: 8,
                kind: BorrowKind::Mutable,
            },
            Op::ActorSendCopy { owner: 1 },
            ret(),
        ]),
        "E_BORROW_CONFLICT"
    );
}

#[test]
fn frozen_publication_requires_deep_sync_witness_and_send() {
    assert_eq!(
        code(vec![
            init(1, false, true),
            Op::PublishFrozen {
                owner: 1,
                region: 9,
                sync_proven: false,
            },
            ret(),
        ]),
        "E_SHARED_FROZEN_PROOF_REQUIRED"
    );
    assert_eq!(
        code(vec![
            init(1, false, false),
            Op::PublishFrozen {
                owner: 1,
                region: 9,
                sync_proven: true,
            },
            ret(),
        ]),
        "E_SHARED_FROZEN_PROOF_REQUIRED"
    );
}

#[test]
fn frozen_publication_consumes_mutable_source_and_rejects_stale_read() {
    let publication = Op::PublishFrozen {
        owner: 1,
        region: 9,
        sync_proven: true,
    };
    assert!(run(vec![init(1, false, true), publication.clone(), ret()]).is_ok());
    assert_eq!(
        code(vec![init(1, false, true), publication, read(1), ret()]),
        "E_USE_AFTER_MOVE_OR_DROP"
    );
}

#[test]
fn frozen_or_guarded_promotion_with_live_borrow_fails_closed() {
    for promotion in [
        Op::PublishFrozen {
            owner: 1,
            region: 20,
            sync_proven: true,
        },
        Op::PromoteGuarded {
            owner: 1,
            region: 21,
        },
    ] {
        assert_eq!(
            code(vec![
                init(1, false, true),
                Op::Borrow {
                    owner: 1,
                    loan: 10,
                    kind: BorrowKind::Shared,
                },
                promotion,
                ret(),
            ]),
            "E_BORROW_CONFLICT"
        );
    }
}

#[test]
fn guarded_region_has_only_runtime_owner_after_promotion() {
    let proof = run(vec![
        init(1, false, true),
        Op::PromoteGuarded {
            owner: 1,
            region: 21,
        },
        ret(),
    ])
    .expect("guarded promotion must consume unique actor-local owner");
    assert!(proof.cleanup_by_exit[&0].is_empty());
    assert_eq!(
        code(vec![
            init(1, false, true),
            Op::PromoteGuarded {
                owner: 1,
                region: 21,
            },
            Op::Write { owner: 1 },
            ret(),
        ]),
        "E_USE_AFTER_MOVE_OR_DROP"
    );
    assert_eq!(
        code(vec![
            init(1, false, false),
            Op::PromoteGuarded {
                owner: 1,
                region: 21,
            },
            ret(),
        ]),
        "E_ACTOR_SEND_PROOF_REQUIRED"
    );
}

#[test]
fn unknown_zero_or_duplicate_region_identity_is_rejected_before_execution() {
    assert_eq!(
        code(vec![
            init(1, false, true),
            Op::PromoteGuarded {
                owner: 1,
                region: 0,
            },
            ret(),
        ]),
        "E_REGION_ID_ZERO"
    );
    assert_eq!(
        code(vec![
            init(1, false, true),
            init(2, false, true),
            Op::PublishFrozen {
                owner: 1,
                region: 13,
                sync_proven: true,
            },
            Op::PromoteGuarded {
                owner: 2,
                region: 13,
            },
            ret(),
        ]),
        "E_REGION_DEF_REUSED"
    );
}

#[test]
fn raw_borrow_still_cannot_cross_actor_boundary() {
    assert_eq!(
        code(vec![
            init(1, false, true),
            Op::ActorHopBorrow { owner: 1 },
            ret(),
        ]),
        "E_ACTOR_BORROW_ESCAPE"
    );
}

#[test]
fn rejected_send_does_not_publish_shadow_success_events() {
    let program = Program {
        entry: 0,
        blocks: vec![Block {
            id: 0,
            operations: vec![
                init(1, false, true),
                Op::ActorSendMove { owner: 1 },
                read(1),
                ret(),
            ],
            successors: vec![],
        }],
    };
    let mut events = Vec::new();
    let result = verify_with_shadow_v0(&program, &mut |event| {
        events.push(event);
        Ok(())
    });
    assert!(matches!(result, Err(e) if e.code == "E_USE_AFTER_MOVE_OR_DROP"));
    assert!(events.is_empty());
}

fn mir(transfer: Op, effect: Effect) -> MirFunction {
    MirFunction {
        version: CONTRACT_VERSION.into(),
        producer: Producer {
            compiler_git_sha: "a".repeat(40),
            source_sha256: "b".repeat(64),
            source_unit_id: 1,
            function_id: 1,
        },
        entry: 0,
        expected_instruction_ids: vec![1, 2, 3],
        blocks: vec![MirBlock {
            id: 0,
            successors: vec![],
            instructions: vec![
                Instruction {
                    instruction_id: 1,
                    source_node_id: 100,
                    effect: Effect::Ownership,
                    operation: init(1, false, true),
                },
                Instruction {
                    instruction_id: 2,
                    source_node_id: 101,
                    effect,
                    operation: transfer,
                },
                Instruction {
                    instruction_id: 3,
                    source_node_id: 102,
                    effect: Effect::Control,
                    operation: ret(),
                },
            ],
        }],
    }
}

#[test]
fn mir_requires_explicit_actor_boundary_effect_for_mail_and_publication() {
    for operation in [
        Op::ActorSendMove { owner: 1 },
        Op::ActorSendCopy { owner: 1 },
        Op::PublishFrozen {
            owner: 1,
            region: 8,
            sync_proven: true,
        },
        Op::PromoteGuarded {
            owner: 1,
            region: 8,
        },
    ] {
        assert_eq!(
            verify_mir(&mir(operation, Effect::Ownership)),
            Err(MirError::Invalid("E_MIR_EFFECT_MISMATCH"))
        );
    }
    for operation in [
        Op::ActorSendMove { owner: 1 },
        Op::PublishFrozen {
            owner: 1,
            region: 8,
            sync_proven: true,
        },
        Op::PromoteGuarded {
            owner: 1,
            region: 8,
        },
    ] {
        assert!(verify_mir(&mir(operation, Effect::ActorBoundary)).is_ok());
    }
}
