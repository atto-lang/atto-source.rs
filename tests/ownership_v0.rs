use atto_borrow_checker::{verify, Block, BorrowKind, Op, Program, Traits};

fn proof(copy: bool, send: bool) -> Traits {
    Traits {
        is_copy: copy,
        is_send: send,
    }
}
fn program(operations: Vec<Op>) -> Program {
    Program {
        entry: 0,
        blocks: vec![Block {
            id: 0,
            operations,
            successors: vec![],
        }],
    }
}
fn run(operations: Vec<Op>) -> Result<atto_borrow_checker::Proof, atto_borrow_checker::Diagnostic> {
    verify(&program(operations))
}
fn code(operations: Vec<Op>) -> &'static str {
    run(operations).unwrap_err().code
}
fn own(owner: u32) -> Op {
    Op::Init {
        owner,
        traits: proof(false, false),
    }
}
fn release(loan: u32) -> Op {
    Op::EndBorrow { loan }
}
fn borrow(owner: u32, loan: u32, kind: BorrowKind) -> Op {
    Op::Borrow { owner, loan, kind }
}
fn read(owner: u32) -> Op {
    Op::Read { owner }
}
fn ret() -> Op {
    Op::Return
}

#[test]
fn read_and_implicit_reverse_acquisition_cleanup() {
    let result = run(vec![own(3), own(7), read(3), ret()]).unwrap();
    assert_eq!(result.cleanup_by_exit[&0], vec![7, 3]);
}
#[test]
fn independent_copy_proof_and_copy_cleanup_order() {
    let result = run(vec![
        Op::Init {
            owner: 1,
            traits: proof(true, false),
        },
        Op::Copy { from: 1, into: 2 },
        read(1),
        read(2),
        ret(),
    ])
    .unwrap();
    assert_eq!(result.cleanup_by_exit[&0], vec![2, 1]);
}
#[test]
fn move_preserves_original_acquisition_history() {
    let result = run(vec![own(1), own(2), Op::Move { from: 1, into: 3 }, ret()]).unwrap();
    assert_eq!(result.cleanup_by_exit[&0], vec![2, 3]);
}
#[test]
fn use_after_move_fails() {
    assert_eq!(
        code(vec![own(1), Op::Move { from: 1, into: 2 }, read(1), ret()]),
        "E_USE_AFTER_MOVE_OR_DROP"
    );
}
#[test]
fn double_drop_fails() {
    assert_eq!(
        code(vec![
            own(1),
            Op::Drop { owner: 1 },
            Op::Drop { owner: 1 },
            ret()
        ]),
        "E_USE_AFTER_MOVE_OR_DROP"
    );
}
#[test]
fn multiple_shared_loans_and_release_are_valid() {
    run(vec![
        own(1),
        borrow(1, 10, BorrowKind::Shared),
        borrow(1, 11, BorrowKind::Shared),
        read(1),
        release(11),
        release(10),
        ret(),
    ])
    .unwrap();
}
#[test]
fn shared_and_mutable_borrow_conflict() {
    assert_eq!(
        code(vec![
            own(1),
            borrow(1, 10, BorrowKind::Shared),
            borrow(1, 11, BorrowKind::Mutable),
            ret()
        ]),
        "E_BORROW_CONFLICT"
    );
}
#[test]
fn second_exclusive_borrow_conflicts() {
    assert_eq!(
        code(vec![
            own(1),
            borrow(1, 10, BorrowKind::Mutable),
            borrow(1, 11, BorrowKind::Mutable),
            ret()
        ]),
        "E_BORROW_CONFLICT"
    );
}
#[test]
fn mutable_loan_requires_authorized_write() {
    assert_eq!(
        code(vec![
            own(1),
            borrow(1, 10, BorrowKind::Shared),
            Op::WriteThrough { owner: 1, loan: 10 },
            ret()
        ]),
        "E_MUT_LOAN_REQUIRED"
    );
}
#[test]
fn writing_via_matching_mutable_loan_is_valid() {
    run(vec![
        own(1),
        borrow(1, 10, BorrowKind::Mutable),
        Op::WriteThrough { owner: 1, loan: 10 },
        release(10),
        ret(),
    ])
    .unwrap();
}
#[test]
fn direct_owner_read_while_mutably_borrowed_fails() {
    assert_eq!(
        code(vec![
            own(1),
            borrow(1, 10, BorrowKind::Mutable),
            read(1),
            ret()
        ]),
        "E_BORROW_CONFLICT"
    );
}
#[test]
fn taking_while_borrowed_fails() {
    assert_eq!(
        code(vec![
            own(1),
            borrow(1, 10, BorrowKind::Shared),
            Op::Move { from: 1, into: 2 },
            ret()
        ]),
        "E_BORROW_CONFLICT"
    );
}
#[test]
fn root_aliasing_is_frontend_canonical_owner_id() {
    // Both projected and root-level accesses are represented as owner 1.
    assert_eq!(
        code(vec![
            own(1),
            borrow(1, 10, BorrowKind::Shared),
            Op::Write { owner: 1 },
            ret()
        ]),
        "E_BORROW_CONFLICT"
    );
}
#[test]
fn send_without_copy_cannot_duplicate_task_capture() {
    assert_eq!(
        code(vec![
            Op::Init {
                owner: 1,
                traits: proof(false, true)
            },
            Op::TaskCaptureCopy { owner: 1 },
            ret()
        ]),
        "E_TASK_CAPTURE_PROOF"
    );
}
#[test]
fn independent_send_capture_by_move_is_valid() {
    run(vec![
        Op::Init {
            owner: 1,
            traits: proof(false, true),
        },
        Op::TaskCaptureMove { owner: 1 },
        ret(),
    ])
    .unwrap();
}
#[test]
fn non_send_cannot_cross_task_capture() {
    assert_eq!(
        code(vec![own(1), Op::TaskCaptureMove { owner: 1 }, ret()]),
        "E_TASK_CAPTURE_PROOF"
    );
}
#[test]
fn suspension_with_loan_is_rejected() {
    assert_eq!(
        code(vec![
            own(1),
            borrow(1, 10, BorrowKind::Shared),
            Op::Suspend,
            ret()
        ]),
        "E_SUSPEND_WITH_LIVE_LOAN"
    );
}
#[test]
fn suspension_after_loan_release_is_valid() {
    run(vec![
        own(1),
        borrow(1, 10, BorrowKind::Shared),
        release(10),
        Op::Suspend,
        ret(),
    ])
    .unwrap();
}
#[test]
fn loan_cannot_escape_return() {
    assert_eq!(
        code(vec![own(1), borrow(1, 10, BorrowKind::Shared), ret()]),
        "E_LOAN_ESCAPE"
    );
}
#[test]
fn actor_hop_raw_borrow_is_rejected() {
    assert_eq!(
        code(vec![own(1), Op::ActorHopBorrow { owner: 1 }, ret()]),
        "E_ACTOR_BORROW_ESCAPE"
    );
}
#[test]
fn independent_branch_read_paths_join_cleanly() {
    let p = Program {
        entry: 0,
        blocks: vec![
            Block {
                id: 0,
                operations: vec![own(1)],
                successors: vec![1, 2],
            },
            Block {
                id: 1,
                operations: vec![read(1)],
                successors: vec![3],
            },
            Block {
                id: 2,
                operations: vec![read(1)],
                successors: vec![3],
            },
            Block {
                id: 3,
                operations: vec![read(1), ret()],
                successors: vec![],
            },
        ],
    };
    assert_eq!(verify(&p).unwrap().cleanup_by_exit[&3], vec![1]);
}
#[test]
fn divergent_move_at_join_is_rejected_without_resurrection() {
    let p = Program {
        entry: 0,
        blocks: vec![
            Block {
                id: 0,
                operations: vec![own(1)],
                successors: vec![1, 2],
            },
            Block {
                id: 1,
                operations: vec![Op::Move { from: 1, into: 3 }],
                successors: vec![3],
            },
            Block {
                id: 2,
                operations: vec![read(1)],
                successors: vec![3],
            },
            Block {
                id: 3,
                operations: vec![ret()],
                successors: vec![],
            },
        ],
    };
    assert_eq!(verify(&p).unwrap_err().code, "E_CFG_JOIN_UNPROVEN");
}
#[test]
fn cycle_without_bounded_loop_proof_is_rejected() {
    let p = Program {
        entry: 0,
        blocks: vec![
            Block {
                id: 0,
                operations: vec![own(1)],
                successors: vec![1],
            },
            Block {
                id: 1,
                operations: vec![read(1)],
                successors: vec![1],
            },
        ],
    };
    assert_eq!(verify(&p).unwrap_err().code, "E_CFG_CYCLE_UNPROVEN");
}
#[test]
fn unreachable_block_is_rejected() {
    let p = Program {
        entry: 0,
        blocks: vec![
            Block {
                id: 0,
                operations: vec![ret()],
                successors: vec![],
            },
            Block {
                id: 1,
                operations: vec![ret()],
                successors: vec![],
            },
        ],
    };
    assert_eq!(verify(&p).unwrap_err().code, "E_CFG_UNREACHABLE");
}
#[test]
fn duplicate_successor_is_rejected() {
    let p = Program {
        entry: 0,
        blocks: vec![
            Block {
                id: 0,
                operations: vec![own(1)],
                successors: vec![1, 1],
            },
            Block {
                id: 1,
                operations: vec![ret()],
                successors: vec![],
            },
        ],
    };
    assert_eq!(verify(&p).unwrap_err().code, "E_CFG_EDGE");
}
#[test]
fn semantic_failures_are_not_a_generic_unsupported_syntax_gate() {
    assert_eq!(
        code(vec![own(1), Op::Copy { from: 1, into: 2 }, ret()]),
        "E_COPY_PROOF_REQUIRED"
    );
}

#[test]
fn owner_id_reused_in_different_branches_is_rejected() {
    // Two independent Init instructions with the same ID yield equal path
    // states. A path-local uniqueness test would accidentally accept the join.
    let p = Program {
        entry: 0,
        blocks: vec![
            Block {
                id: 0,
                operations: vec![],
                successors: vec![1, 2],
            },
            Block {
                id: 1,
                operations: vec![own(1)],
                successors: vec![3],
            },
            Block {
                id: 2,
                operations: vec![own(1)],
                successors: vec![3],
            },
            Block {
                id: 3,
                operations: vec![ret()],
                successors: vec![],
            },
        ],
    };
    assert_eq!(verify(&p).unwrap_err().code, "E_OWNER_DEF_REUSED");
}

#[test]
fn move_target_reused_in_different_branches_is_rejected() {
    let p = Program {
        entry: 0,
        blocks: vec![
            Block {
                id: 0,
                operations: vec![own(1)],
                successors: vec![1, 2],
            },
            Block {
                id: 1,
                operations: vec![Op::Move { from: 1, into: 2 }],
                successors: vec![3],
            },
            Block {
                id: 2,
                operations: vec![Op::Move { from: 1, into: 2 }],
                successors: vec![3],
            },
            Block {
                id: 3,
                operations: vec![ret()],
                successors: vec![],
            },
        ],
    };
    assert_eq!(verify(&p).unwrap_err().code, "E_OWNER_DEF_REUSED");
}

#[test]
fn loan_id_reused_in_different_branches_is_rejected() {
    let p = Program {
        entry: 0,
        blocks: vec![
            Block {
                id: 0,
                operations: vec![own(1)],
                successors: vec![1, 2],
            },
            Block {
                id: 1,
                operations: vec![borrow(1, 10, BorrowKind::Shared), release(10)],
                successors: vec![3],
            },
            Block {
                id: 2,
                operations: vec![borrow(1, 10, BorrowKind::Shared), release(10)],
                successors: vec![3],
            },
            Block {
                id: 3,
                operations: vec![ret()],
                successors: vec![],
            },
        ],
    };
    assert_eq!(verify(&p).unwrap_err().code, "E_LOAN_DEF_REUSED");
}

#[test]
fn zero_owner_identity_is_rejected() {
    assert_eq!(code(vec![own(0), ret()]), "E_OWNER_ID_ZERO");
    assert_eq!(
        code(vec![own(1), Op::Move { from: 1, into: 0 }, ret()]),
        "E_OWNER_ID_ZERO"
    );
}

#[test]
fn zero_loan_identity_is_rejected() {
    assert_eq!(
        code(vec![
            own(1),
            borrow(1, 0, BorrowKind::Shared),
            release(0),
            ret()
        ]),
        "E_LOAN_ID_ZERO"
    );
}
