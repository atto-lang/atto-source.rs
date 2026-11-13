//! Suspension CFG regression tests. These prove bounded, stable resume states
//! for synchronous generators; guest Wasm continuation emission is a separate
//! milestone and deliberately remains E_UNSUPPORTED until implemented.

use atto_compiler::ast::{Expr, ExprKind, Stmt};
use atto_compiler::diag::Pos;
use atto_compiler::generator_cfg::{lower, Instruction, MAX_GENERATOR_INSTRUCTIONS};
use atto_compiler::{lexer::lex, parser::Parser};

fn first_generator(source: &str) -> Vec<Stmt> {
    let program = Parser::new(lex(source).unwrap()).parse_program().unwrap();
    assert!(program.funcs[0].is_generator);
    program.funcs[0].body.clone()
}

#[test]
fn yield_points_resume_in_source_order_and_end_at_done() {
    let body = first_generator(r#"
        fnc *gen values(): Generator<int> {
            yield 20;
            yield 22;
        }
    "#);
    let plan = lower(&body).unwrap();
    assert_eq!(plan.yields, 2);
    assert_eq!(plan.delegate_slots, 0);
    assert_eq!(plan.instructions.len(), 3);
    assert!(matches!(plan.instructions[0], Instruction::Yield { .. }));
    assert!(matches!(plan.instructions[1], Instruction::Yield { .. }));
    assert!(matches!(plan.instructions[2], Instruction::Done));
}

#[test]
fn loop_and_yield_star_lower_to_distinct_pull_driven_resume_states() {
    let program = Parser::new(lex(r#"
        fnc *gen count_to(int limit): Generator<int> {
            for (let i = 1; i <= limit; i++) do
                yield i;
            done
        }
        fnc *gen count_with_zero(int limit): Generator<int> {
            yield 0;
            yield* count_to(limit);
        }
    "#).unwrap()).parse_program().unwrap();
    let leaf = lower(&program.funcs[0].body).unwrap();
    assert_eq!(leaf.yields, 1);
    assert_eq!(leaf.delegate_slots, 0);
    assert!(matches!(leaf.instructions.first(), Some(Instruction::Evaluate(Stmt::Bind { .. }))));
    assert!(matches!(leaf.instructions[1], Instruction::Guard { .. }));
    assert!(matches!(leaf.instructions[2], Instruction::Yield { .. }));
    assert!(matches!(leaf.instructions[3], Instruction::Evaluate(Stmt::Assign { .. })));
    assert!(matches!(leaf.instructions[4], Instruction::Jump { to: 1 }));
    assert!(matches!(leaf.instructions[5], Instruction::Done));

    let outer = lower(&program.funcs[1].body).unwrap();
    assert_eq!(outer.yields, 1);
    assert_eq!(outer.delegate_slots, 1);
    assert!(matches!(outer.instructions[0], Instruction::Yield { .. }));
    assert!(matches!(&outer.instructions[1], Instruction::Delegate { source, slot: 0 }
        if matches!(&source.kind, ExprKind::Call(name, _) if name == "count_to")));
    assert!(matches!(outer.instructions[2], Instruction::Done));
}

#[test]
fn branch_and_nested_loop_jumps_are_in_bounds_and_break_continue_have_own_targets() {
    let body = first_generator(r#"
        fnc *gen g(int limit): Generator<int> {
            for (let i = 0; i < limit; i++) do
                if i eq 1 then continue; fi
                if i eq 2 then break; fi
                yield i;
            done
            yield 42;
        }
    "#);
    let plan = lower(&body).unwrap();
    let size = plan.instructions.len() as u32;
    let mut guards = 0;
    let mut jumps = 0;
    let mut backward = 0;
    for (pc, op) in plan.instructions.iter().enumerate() {
        match op {
            Instruction::Guard { on_false, .. } => {
                assert!(*on_false < size, "guard {pc} escapes CFG");
                guards += 1;
            }
            Instruction::Jump { to } => {
                assert!(*to < size, "jump {pc} escapes CFG");
                if *to <= pc as u32 { backward += 1; }
                jumps += 1;
            }
            _ => {}
        }
    }
    assert!(guards >= 3, "loop condition and both if branches");
    assert!(jumps >= 4, "continue, break, if endings, backedge");
    assert!(backward >= 1, "loop must have a backedge");
    assert_eq!(plan.yields, 2);
}

#[test]
fn delegates_have_distinct_frame_slots_even_across_conditional_arms() {
    let body = first_generator(r#"
        fnc *gen g(bool select): Generator<int> {
            if select then
                yield* first();
            else
                yield* second();
            fi
            yield* third();
        }
    "#);
    let plan = lower(&body).unwrap();
    assert_eq!(plan.delegate_slots, 3);
    let slots: Vec<_> = plan.instructions.iter().filter_map(|i|
        if let Instruction::Delegate { slot, .. } = i { Some(*slot) } else { None }
    ).collect();
    assert_eq!(slots, vec![0, 1, 2]);
    assert!(matches!(plan.instructions.last(), Some(Instruction::Done)));
}

#[test]
fn cfg_is_bounded_and_does_not_silently_accept_unlowered_for_in() {
    let pos = Pos { line: 7, col: 2 };
    let one = Stmt::Yield(Expr { kind: ExprKind::Int(Some(1)), pos }, pos);
    let too_many = vec![one; MAX_GENERATOR_INSTRUCTIONS];
    let err = lower(&too_many).unwrap_err();
    assert_eq!(err.code, "E_SOURCE_LIMIT");
    let body = first_generator(r#"
        fnc *gen g(): Generator<int> {
            for value in other() do yield value; done
        }
    "#);
    let err = lower(&body).unwrap_err();
    assert_eq!(err.code, "E_UNSUPPORTED");
    assert!(err.message.contains("for-in"));
}

#[test]
fn cfg_is_deterministic_and_keeps_guest_compilation_fail_closed() {
    let src = r#"
        fnc *gen count(): Generator<int> { yield 1; yield 2; }
        pub fnc main(): int { return 0; }
    "#;
    let body = first_generator(src);
    let a = format!("{:?}", lower(&body).unwrap());
    let b = format!("{:?}", lower(&body).unwrap());
    assert_eq!(a, b);
    let diagnostic = atto_compiler::compile(src).unwrap_err();
    assert_eq!(diagnostic.code, "E_UNSUPPORTED");
    assert!(diagnostic.message.contains("continuation"));
}
