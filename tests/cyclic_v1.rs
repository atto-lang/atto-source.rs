use atto_borrow_checker::cyclic_v1::{check, Block, Code, LoanKind, Op, Place, Program, Root};

fn root() -> Place {
    Place::root(1)
}
fn loan(id: u32, kind: LoanKind) -> Op {
    Op::Borrow {
        loan: id,
        place: root(),
        kind,
    }
}
fn graph(blocks: Vec<Block>) -> Program {
    Program {
        roots: vec![Root {
            id: 1,
            is_copy: false,
            is_transferable: true,
        }],
        entry: 0,
        blocks,
    }
}
fn block(id: u32, ops: Vec<Op>, successors: &[u32]) -> Block {
    Block {
        id,
        ops,
        successors: successors.to_vec(),
    }
}
fn rejects(ir: Program, code: Code) {
    assert_eq!(check(&ir).unwrap_err().code, code);
}

#[test]
fn readers_can_overlap_but_cannot_write_through_their_loans() {
    check(&graph(vec![block(
        0,
        vec![
            loan(10, LoanKind::Shared),
            loan(11, LoanKind::Shared),
            Op::ReadThrough(10),
            Op::ReadThrough(11),
            Op::EndLoan(10),
            Op::EndLoan(11),
        ],
        &[],
    )]))
    .unwrap();
    rejects(
        graph(vec![block(
            0,
            vec![loan(10, LoanKind::Shared), Op::WriteThrough(10)],
            &[],
        )]),
        Code::LoanConflict,
    );
}

#[test]
fn exclusive_loan_can_read_and_write_without_raw_owner_access() {
    check(&graph(vec![block(
        0,
        vec![
            loan(10, LoanKind::Exclusive),
            Op::ReadThrough(10),
            Op::WriteThrough(10),
            Op::EndLoan(10),
            Op::Read(root()),
        ],
        &[],
    )]))
    .unwrap();
    rejects(
        graph(vec![block(
            0,
            vec![loan(10, LoanKind::Exclusive), Op::Write(root())],
            &[],
        )]),
        Code::LoanConflict,
    );
}

#[test]
fn ended_or_missing_loans_cannot_authorize_reads_or_writes() {
    for access in [Op::ReadThrough(10), Op::WriteThrough(10)] {
        rejects(
            graph(vec![block(
                0,
                vec![loan(10, LoanKind::Exclusive), Op::EndLoan(10), access],
                &[],
            )]),
            Code::LoanNotLive,
        );
    }
}

#[test]
fn every_loop_iteration_releases_and_reacquires_its_exclusive_loan() {
    check(&graph(vec![
        block(0, vec![], &[1]),
        block(
            1,
            vec![
                loan(10, LoanKind::Exclusive),
                Op::WriteThrough(10),
                Op::EndLoan(10),
            ],
            &[1, 2],
        ),
        block(2, vec![Op::Transfer(root())], &[]),
    ]))
    .unwrap();
}

#[test]
fn backedge_move_cannot_be_hidden_by_the_initial_entry_state() {
    rejects(
        graph(vec![
            block(0, vec![], &[1]),
            block(1, vec![Op::Read(root()), Op::Move(root())], &[1, 2]),
            block(2, vec![], &[]),
        ]),
        Code::UseAfterMove,
    );
}

#[test]
fn exact_move_reinitialization_is_valid_on_every_iteration() {
    check(&graph(vec![
        block(0, vec![], &[1]),
        block(1, vec![Op::Move(root()), Op::Reinitialize(root())], &[1, 2]),
        block(2, vec![Op::Read(root())], &[]),
    ]))
    .unwrap();
}

#[test]
fn backedge_cannot_reacquire_a_still_live_loan() {
    rejects(
        graph(vec![
            block(0, vec![], &[1]),
            block(1, vec![loan(10, LoanKind::Shared)], &[1, 2]),
            block(2, vec![Op::EndLoan(10)], &[]),
        ]),
        Code::LoanConflict,
    );
}

#[test]
fn join_rechecks_late_predecessors_before_accepting_a_write() {
    for successors in [[1, 2], [2, 1]] {
        rejects(
            graph(vec![
                block(0, vec![], &successors),
                block(1, vec![], &[3]),
                block(2, vec![loan(10, LoanKind::Shared)], &[4]),
                block(4, vec![], &[3]),
                block(3, vec![Op::Write(root())], &[]),
            ]),
            Code::LoanConflict,
        );
    }
}

#[test]
fn write_loan_must_be_present_on_all_paths() {
    rejects(
        graph(vec![
            block(0, vec![], &[1, 2]),
            block(1, vec![loan(10, LoanKind::Exclusive)], &[3]),
            block(2, vec![], &[3]),
            block(3, vec![Op::WriteThrough(10)], &[]),
        ]),
        Code::LoanNotLive,
    );
}

#[test]
fn mutable_sibling_fields_are_disjoint_but_parent_access_conflicts() {
    let first = Op::Borrow {
        loan: 10,
        place: root().field(1),
        kind: LoanKind::Exclusive,
    };
    let second = Op::Borrow {
        loan: 11,
        place: root().field(2),
        kind: LoanKind::Exclusive,
    };
    check(&graph(vec![block(
        0,
        vec![
            first.clone(),
            second,
            Op::WriteThrough(10),
            Op::WriteThrough(11),
            Op::EndLoan(10),
            Op::EndLoan(11),
        ],
        &[],
    )]))
    .unwrap();
    rejects(
        graph(vec![block(0, vec![first, Op::Read(root())], &[])]),
        Code::LoanConflict,
    );
}

#[test]
fn nested_field_prefix_is_not_a_disjoint_sibling() {
    rejects(
        graph(vec![block(
            0,
            vec![
                Op::Borrow {
                    loan: 10,
                    place: root().field(1),
                    kind: LoanKind::Shared,
                },
                Op::Write(root().field(1).field(2)),
            ],
            &[],
        )]),
        Code::LoanConflict,
    );
}

#[test]
fn suspension_cannot_retain_even_a_read_only_loan() {
    rejects(
        graph(vec![block(
            0,
            vec![loan(10, LoanKind::Shared), Op::Suspend],
            &[],
        )]),
        Code::UnsupportedSuspension,
    );
}
