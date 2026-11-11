use atto_borrow_checker::mir_v1::{
    verify_mir, Effect, Instruction, MirBlock, MirError, MirFunction, Producer, CONTRACT_VERSION,
};
use atto_borrow_checker::{
    verify, verify_with_shadow_v0, Block, BorrowKind, Op, Program, Traits,
};

fn init(owner: u32, send: bool) -> Op {
    Op::Init {
        owner,
        traits: Traits {
            is_copy: false,
            is_send: send,
        },
    }
}

fn linear(operations: Vec<Op>) -> Program {
    Program {
        entry: 0,
        blocks: vec![Block {
            id: 0,
            operations,
            successors: vec![],
        }],
    }
}

fn rejected(operations: Vec<Op>) -> &'static str {
    verify(&linear(operations)).unwrap_err().code
}

#[test]
fn committed_and_aborted_paths_have_distinct_single_owner_cleanup() {
    let program = Program {
        entry: 0,
        blocks: vec![
            Block {
                id: 0,
                operations: vec![
                    init(1, true),
                    Op::ActorSendPrepare {
                        owner: 1,
                        ticket: 9,
                    },
                ],
                successors: vec![1, 2],
            },
            Block {
                id: 1,
                operations: vec![Op::ActorSendCommit { ticket: 9 }, Op::Return],
                successors: vec![],
            },
            Block {
                id: 2,
                operations: vec![
                    Op::ActorSendAbort { ticket: 9 },
                    Op::Read { owner: 1 },
                    Op::Return,
                ],
                successors: vec![],
            },
        ],
    };
    let proof = verify(&program).expect("commit and rollback each preserve one owner");
    assert!(proof.cleanup_by_exit[&1].is_empty());
    assert_eq!(proof.cleanup_by_exit[&2], vec![1]);
}

#[test]
fn pending_send_seals_all_local_aliasing_and_cross_boundary_operations() {
    let pending = Op::ActorSendPrepare {
        owner: 1,
        ticket: 9,
    };
    for operation in [
        Op::Read { owner: 1 },
        Op::Write { owner: 1 },
        Op::Move { from: 1, into: 2 },
        Op::Copy { from: 1, into: 2 },
        Op::Drop { owner: 1 },
        Op::TaskCaptureMove { owner: 1 },
        Op::ActorSendCopy { owner: 1 },
        Op::ActorSendMove { owner: 1 },
        Op::ActorSendPrepare {
            owner: 1,
            ticket: 10,
        },
        Op::PublishFrozen {
            owner: 1,
            region: 3,
            sync_proven: true,
        },
        Op::PromoteGuarded {
            owner: 1,
            region: 4,
        },
    ] {
        assert_eq!(
            rejected(vec![
                init(1, true),
                pending.clone(),
                operation,
                Op::Return,
            ]),
            "E_ACTOR_SEND_PENDING"
        );
    }
}

#[test]
fn abort_restores_sender_and_commit_fences_stale_sender() {
    assert!(verify(&linear(vec![
        init(1, true),
        Op::ActorSendPrepare {
            owner: 1,
            ticket: 9,
        },
        Op::ActorSendAbort { ticket: 9 },
        Op::Write { owner: 1 },
        Op::Return,
    ]))
    .is_ok());

    assert_eq!(
        rejected(vec![
            init(1, true),
            Op::ActorSendPrepare {
                owner: 1,
                ticket: 9,
            },
            Op::ActorSendCommit { ticket: 9 },
            Op::Read { owner: 1 },
            Op::Return,
        ]),
        "E_USE_AFTER_MOVE_OR_DROP"
    );
}

#[test]
fn admission_requires_transitive_send_and_no_outstanding_loan() {
    assert_eq!(
        rejected(vec![
            init(1, false),
            Op::ActorSendPrepare {
                owner: 1,
                ticket: 1,
            },
            Op::Return,
        ]),
        "E_ACTOR_SEND_PROOF_REQUIRED"
    );

    assert_eq!(
        rejected(vec![
            init(1, true),
            Op::Borrow {
                owner: 1,
                loan: 2,
                kind: BorrowKind::Shared,
            },
            Op::ActorSendPrepare {
                owner: 1,
                ticket: 1,
            },
            Op::Return,
        ]),
        "E_BORROW_CONFLICT"
    );
}

#[test]
fn unresolved_double_settle_and_unknown_tickets_fail_closed() {
    assert_eq!(
        rejected(vec![
            init(1, true),
            Op::ActorSendPrepare {
                owner: 1,
                ticket: 9,
            },
            Op::Return,
        ]),
        "E_ACTOR_SEND_UNRESOLVED"
    );
    assert_eq!(
        rejected(vec![init(1, true), Op::ActorSendCommit { ticket: 9 }, Op::Return]),
        "E_UNKNOWN_ACTOR_SEND_TICKET"
    );
    assert_eq!(
        rejected(vec![init(1, true), Op::ActorSendAbort { ticket: 9 }, Op::Return]),
        "E_UNKNOWN_ACTOR_SEND_TICKET"
    );
    assert_eq!(
        rejected(vec![
            init(1, true),
            Op::ActorSendPrepare {
                owner: 1,
                ticket: 9,
            },
            Op::ActorSendAbort { ticket: 9 },
            Op::ActorSendCommit { ticket: 9 },
            Op::Return,
        ]),
        "E_UNKNOWN_ACTOR_SEND_TICKET"
    );
}

#[test]
fn zero_and_reused_ticket_identity_are_rejected_globally() {
    assert_eq!(
        rejected(vec![
            init(1, true),
            Op::ActorSendPrepare {
                owner: 1,
                ticket: 0,
            },
            Op::Return,
        ]),
        "E_SEND_TICKET_ID_ZERO"
    );

    let program = Program {
        entry: 0,
        blocks: vec![
            Block {
                id: 0,
                operations: vec![init(1, true), init(2, true)],
                successors: vec![1, 2],
            },
            Block {
                id: 1,
                operations: vec![
                    Op::ActorSendPrepare {
                        owner: 1,
                        ticket: 9,
                    },
                    Op::ActorSendCommit { ticket: 9 },
                    Op::Return,
                ],
                successors: vec![],
            },
            Block {
                id: 2,
                operations: vec![
                    Op::ActorSendPrepare {
                        owner: 2,
                        ticket: 9,
                    },
                    Op::ActorSendAbort { ticket: 9 },
                    Op::Return,
                ],
                successors: vec![],
            },
        ],
    };
    assert_eq!(
        verify(&program).unwrap_err().code,
        "E_SEND_TICKET_DEF_REUSED"
    );
}

#[test]
fn an_async_backpressure_suspend_keeps_owner_in_escrow() {
    let program = linear(vec![
        init(1, true),
        Op::ActorSendPrepare {
            owner: 1,
            ticket: 9,
        },
        Op::Suspend,
        Op::ActorSendAbort { ticket: 9 },
        Op::Read { owner: 1 },
        Op::Return,
    ]);
    assert_eq!(
        verify(&program).unwrap().cleanup_by_exit[&0],
        vec![1],
        "suspension does not grant sender access while the ticket is pending"
    );
}

#[test]
fn invalid_late_settlement_never_publishes_tentative_shadow_events() {
    let program = linear(vec![
        init(1, true),
        Op::ActorSendPrepare {
            owner: 1,
            ticket: 9,
        },
        Op::ActorSendAbort { ticket: 9 },
        Op::ActorSendCommit { ticket: 9 },
        Op::Return,
    ]);
    let mut observed = Vec::new();
    assert!(verify_with_shadow_v0(&program, &mut |event| {
        observed.push(event);
        Ok(())
    })
    .is_err());
    assert!(observed.is_empty());
}

#[test]
fn typed_mir_requires_actor_boundary_effect_for_all_transaction_phases() {
    for operation in [
        Op::ActorSendPrepare {
            owner: 1,
            ticket: 9,
        },
        Op::ActorSendCommit { ticket: 9 },
        Op::ActorSendAbort { ticket: 9 },
    ] {
        let function = MirFunction {
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
                        source_node_id: 101,
                        effect: Effect::Ownership,
                        operation: init(1, true),
                    },
                    Instruction {
                        instruction_id: 2,
                        source_node_id: 102,
                        effect: Effect::Ownership,
                        operation,
                    },
                    Instruction {
                        instruction_id: 3,
                        source_node_id: 103,
                        effect: Effect::Control,
                        operation: Op::Return,
                    },
                ],
            }],
        };
        assert_eq!(
            verify_mir(&function),
            Err(MirError::Invalid("E_MIR_EFFECT_MISMATCH"))
        );
    }
}
