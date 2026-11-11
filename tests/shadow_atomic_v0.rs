use atto_borrow_checker::{
    verify, verify_with_shadow_v0, Block, Diagnostic, Op, Program, ShadowEventV0, Traits,
};

fn initialized() -> Op {
    Op::Init {
        owner: 1,
        traits: Traits {
            is_copy: false,
            is_send: false,
        },
    }
}

#[test]
fn invalid_later_block_never_publishes_earlier_accepted_events() {
    let p = Program {
        entry: 0,
        blocks: vec![
            Block {
                id: 0,
                operations: vec![initialized(), Op::Read { owner: 1 }],
                successors: vec![1],
            },
            Block {
                id: 1,
                operations: vec![Op::Drop { owner: 1 }, Op::Read { owner: 1 }, Op::Return],
                successors: vec![],
            },
        ],
    };
    let mut published = vec![];
    let failure = verify_with_shadow_v0(&p, &mut |event| {
        published.push(event);
        Ok(())
    })
    .unwrap_err();
    assert_eq!(failure.code, "E_USE_AFTER_MOVE_OR_DROP");
    assert!(
        published.is_empty(),
        "pre-proof operation events must not leak"
    );
}

#[test]
fn invalid_cfg_join_never_publishes_events() {
    let p = Program {
        entry: 0,
        blocks: vec![
            Block {
                id: 0,
                operations: vec![initialized()],
                successors: vec![1, 2],
            },
            Block {
                id: 1,
                operations: vec![Op::Move { from: 1, into: 2 }],
                successors: vec![3],
            },
            Block {
                id: 2,
                operations: vec![Op::Read { owner: 1 }],
                successors: vec![3],
            },
            Block {
                id: 3,
                operations: vec![Op::Return],
                successors: vec![],
            },
        ],
    };
    let mut published = vec![];
    assert_eq!(
        verify_with_shadow_v0(&p, &mut |event| {
            published.push(event);
            Ok(())
        })
        .unwrap_err()
        .code,
        "E_CFG_JOIN_UNPROVEN"
    );
    assert!(published.is_empty());
}

#[test]
fn successful_trace_is_deterministic_and_identical_to_unobserved_proof() {
    let p = Program {
        entry: 0,
        blocks: vec![
            Block {
                id: 0,
                operations: vec![initialized()],
                successors: vec![1, 2],
            },
            Block {
                id: 1,
                operations: vec![Op::Read { owner: 1 }],
                successors: vec![3],
            },
            Block {
                id: 2,
                operations: vec![Op::Read { owner: 1 }],
                successors: vec![3],
            },
            Block {
                id: 3,
                operations: vec![Op::Return],
                successors: vec![],
            },
        ],
    };
    let plain = verify(&p).unwrap();
    let mut a = Vec::<ShadowEventV0>::new();
    let mut b = Vec::<ShadowEventV0>::new();
    let proof_a = verify_with_shadow_v0(&p, &mut |ev| {
        a.push(ev);
        Ok(())
    })
    .unwrap();
    let proof_b = verify_with_shadow_v0(&p, &mut |ev| {
        b.push(ev);
        Ok(())
    })
    .unwrap();
    assert_eq!(plain, proof_a);
    assert_eq!(proof_a, proof_b);
    assert_eq!(a, b);
    assert_eq!(a.len(), 4);
    assert_eq!(a[3].kind, "RETURN");
}

#[test]
fn shadow_limit_fails_closed_but_untraced_fast_path_remains_available() {
    // 66 * 1024 events = 67584 events, beyond the 65536 shadow-only budget.
    // Graph, operation and owner budgets themselves remain valid.
    let mut blocks = Vec::new();
    for id in 0..66u32 {
        let mut operations = vec![Op::Read { owner: 1 }; 1024];
        if id == 0 {
            operations[0] = initialized();
        }
        let successors = if id == 65 {
            operations[1023] = Op::Return;
            vec![]
        } else {
            vec![id + 1]
        };
        blocks.push(Block {
            id,
            operations,
            successors,
        });
    }
    let p = Program { entry: 0, blocks };
    assert!(
        verify(&p).is_ok(),
        "ordinary fast path must not inherit trace cap"
    );
    let mut delivered = 0usize;
    let error = verify_with_shadow_v0(&p, &mut |_event| {
        delivered += 1;
        Ok(())
    })
    .unwrap_err();
    assert_eq!(error.code, "E_SHADOW_EVENT_BUDGET");
    assert_eq!(delivered, 0, "overflow must publish no partial trace");
}

#[test]
fn consumer_error_propagates_only_after_full_cfg_proof() {
    let p = Program {
        entry: 0,
        blocks: vec![Block {
            id: 0,
            operations: vec![initialized(), Op::Return],
            successors: vec![],
        }],
    };
    let mut calls = 0u32;
    let error = verify_with_shadow_v0(&p, &mut |_ev| {
        calls += 1;
        Err(Diagnostic {
            code: "E_SHADOW_CONSUMER",
            block: 0,
            operation: 0,
        })
    })
    .unwrap_err();
    assert_eq!(error.code, "E_SHADOW_CONSUMER");
    assert_eq!(calls, 1);
}
