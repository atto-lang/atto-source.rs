use atto_borrow_checker::{
    verify, verify_with_shadow_v0, Block, Diagnostic, Op, Program, ShadowEventV0, Traits,
};

fn graph(ops: Vec<Op>) -> Program {
    Program {
        entry: 0,
        blocks: vec![Block {
            id: 0,
            operations: ops,
            successors: vec![],
        }],
    }
}

#[test]
fn shadow_reports_only_accepted_inpass_operations() {
    let p = graph(vec![
        Op::Init {
            owner: 7,
            traits: Traits {
                is_copy: false,
                is_send: false,
            },
        },
        Op::Read { owner: 7 },
        Op::Return,
    ]);
    let expected = verify(&p).unwrap();
    let mut events: Vec<ShadowEventV0> = vec![];
    let traced = verify_with_shadow_v0(&p, &mut |ev| {
        events.push(ev);
        Ok(())
    })
    .unwrap();
    assert_eq!(traced, expected);
    assert_eq!(events.len(), 3);
    assert_eq!(events[0].kind, "INIT");
    assert_eq!(events[0].owner, Some(7));
    assert_eq!(events[0].block, 0);
    assert_eq!(events[0].operation, 0);
    assert_eq!(events[1].kind, "READ");
    assert_eq!(events[2].kind, "RETURN");
}

#[test]
fn rejected_operation_is_not_recorded_as_accepted() {
    let p = graph(vec![Op::Read { owner: 7 }, Op::Return]);
    let mut events = vec![];
    let error = verify_with_shadow_v0(&p, &mut |ev| {
        events.push(ev);
        Ok(())
    })
    .unwrap_err();
    assert_eq!(error.code, "E_USE_AFTER_MOVE_OR_DROP");
    assert!(events.is_empty());
}

#[test]
fn consumer_error_prevents_a_successful_proof() {
    let p = graph(vec![Op::Return]);
    let denied = verify_with_shadow_v0(&p, &mut |_event| {
        Err(Diagnostic {
            code: "E_SHADOW_CONSUMER",
            block: 0,
            operation: 0,
        })
    })
    .unwrap_err();
    assert_eq!(denied.code, "E_SHADOW_CONSUMER");
}

#[test]
fn return_cannot_be_repeated_or_precede_operations() {
    let two = graph(vec![Op::Return, Op::Return]);
    assert_eq!(verify(&two).unwrap_err().code, "E_IMPLICIT_EXIT");
    let early = graph(vec![Op::Return, Op::Read { owner: 7 }]);
    assert_eq!(verify(&early).unwrap_err().code, "E_IMPLICIT_EXIT");
}
