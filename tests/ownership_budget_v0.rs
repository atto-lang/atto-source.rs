use atto_borrow_checker::{verify, verify_with_shadow_v0, Block, Op, Program, Traits};

fn oversized_operations() -> Program {
    Program {
        entry: 0,
        blocks: vec![Block {
            id: 0,
            operations: vec![
                Op::Init {
                    owner: 0,
                    traits: Traits {
                        is_copy: false,
                        is_send: false
                    }
                };
                1025
            ],
            successors: vec![],
        }],
    }
}

#[test]
fn block_budget_precedes_identity_scanning() {
    assert_eq!(
        verify(&oversized_operations()).unwrap_err().code,
        "E_CFG_BUDGET"
    );
}

#[test]
fn successor_budget_precedes_identity_scanning() {
    let program = Program {
        entry: 0,
        blocks: vec![Block {
            id: 0,
            operations: vec![Op::Init {
                owner: 0,
                traits: Traits {
                    is_copy: false,
                    is_send: false,
                },
            }],
            successors: vec![0; 16385],
        }],
    };
    assert_eq!(verify(&program).unwrap_err().code, "E_CFG_BUDGET");
}

#[test]
fn cumulative_edge_budget_precedes_graph_scanning() {
    let program = Program {
        entry: 0,
        blocks: vec![
            Block {
                id: 0,
                operations: vec![Op::Return],
                successors: vec![1; 8193],
            },
            Block {
                id: 1,
                operations: vec![Op::Return],
                successors: vec![0; 8193],
            },
        ],
    };
    assert_eq!(verify(&program).unwrap_err().code, "E_CFG_BUDGET");
}

#[test]
fn rejected_oversized_cfg_never_calls_shadow_sink() {
    let mut observed = 0;
    let error = verify_with_shadow_v0(&oversized_operations(), &mut |_| {
        observed += 1;
        Ok(())
    })
    .unwrap_err();
    assert_eq!(observed, 0);
    assert_eq!(error.code, "E_CFG_BUDGET");
}

#[test]
fn exact_block_operation_limit_remains_admitted() {
    let mut operations = vec![Op::Init {
        owner: 1,
        traits: Traits {
            is_copy: false,
            is_send: false,
        },
    }];
    operations.extend(vec![Op::Read { owner: 1 }; 1022]);
    operations.push(Op::Return);
    let program = Program {
        entry: 0,
        blocks: vec![Block {
            id: 0,
            operations,
            successors: vec![],
        }],
    };
    assert_eq!(verify(&program).unwrap().cleanup_by_exit[&0], vec![1]);
}
