//! Typed suspension CFG admission, persistent slot and safety-gate tests.
//! Actual guest-Wasm execution and compiler byte parity are exercised by
//! tests/generator-execution.mjs on the exact-source private Actions runner.

use atto_compiler::{
    check::{check, TGenStep},
    lexer::lex,
    parser::Parser,
};

fn program(src: &str) -> atto_compiler::check::TProgram {
    let ast = Parser::new(lex(src).expect("lex")).parse_program().expect("parse");
    check(&ast).expect("typed compilation must succeed")
}

const RUNNING_TOTALS: &str = r#"
fnc *gen running_totals(int limit): Generator<int> {
  let count = 0;
  let total = 0;
  while count < limit do
    count += 1;
    total += count;
    yield total;
  done
}
pub fnc main(): int {
  let sum = 0;
  for value in running_totals(3) do sum += value; done
  return sum;
}
"#;

#[test]
fn multiple_live_locals_have_stable_slots_and_real_cfg_backedges() {
    let typed = program(RUNNING_TOTALS);
    assert_eq!(typed.generators.len(), 1);
    let generator = &typed.generators[0];
    assert_eq!(generator.locals, 2, "count and total need separate persistent slots");
    let mut set_slots = Vec::new();
    let mut guards = 0;
    let mut jumps = 0;
    let mut yields = 0;
    for (pc, step) in generator.steps.iter().enumerate() {
        match step {
            TGenStep::Set { local, .. } => set_slots.push(*local),
            TGenStep::Guard { on_false, .. } => {
                assert!((*on_false as usize) < generator.steps.len(), "{pc} guard out of bounds");
                guards += 1;
            }
            TGenStep::Jump { to } => {
                assert!((*to as usize) < generator.steps.len(), "{pc} jump out of bounds");
                jumps += 1;
            }
            TGenStep::Yield(_) => yields += 1,
            _ => {}
        }
    }
    assert_eq!(set_slots, vec![0, 1, 0, 1]);
    assert_eq!(guards, 1);
    assert_eq!(jumps, 1);
    assert_eq!(yields, 1);
    assert!(matches!(generator.steps.last(), Some(TGenStep::Done)));
}

#[test]
fn generic_generator_wasm_compiler_admits_multiple_live_locals() {
    let wasm = atto_compiler::compile(RUNNING_TOTALS)
        .expect("two mutable ints must compile to a real guest module");
    assert!(wasm.starts_with(b"\0asm\x01\0\0\0"));
    assert!(!wasm.is_empty());
}

#[test]
fn typed_generator_rejects_scope_escape_immutable_mutation_and_wrong_types() {
    let cases = [
        (r#"fnc *gen bad(): Generator<int> {
              let a = true;
              yield 2;
            }
            pub fnc main(): int { return 0; }"#, "E_TYPE"),
        (r#"fnc *gen bad(): Generator<int> {
              val a = 2;
              a += 1;
              yield a;
            }
            pub fnc main(): int { return 0; }"#, "E_IMMUTABLE"),
        (r#"fnc *gen bad(): Generator<int> {
              let a = 2;
              let a = 3;
              yield a;
            }
            pub fnc main(): int { return 0; }"#, "E_DUPLICATE"),
        (r#"fnc *gen bad(): Generator<int> {
              let a = 2;
              if true then let b = 3; fi
              yield a;
            }
            pub fnc main(): int { return 0; }"#, "E_UNSUPPORTED"),
        (r#"fnc *gen bad(): Generator<int> {
              let a = 2;
              yield a / 3;
            }
            pub fnc main(): int { return 0; }"#, "E_UNSUPPORTED"),
    ];
    for (src, expected) in cases {
        let e = atto_compiler::compile(src).expect_err("unadmitted generator must fail closed");
        assert_eq!(e.code, expected, "{src}: {e}");
    }
}

#[test]
fn fixed_frame_slot_limit_blocks_oversized_live_local_sets() {
    let mut source = String::from("fnc *gen g(): Generator<int> {\n");
    for i in 0..17 {
        source.push_str(&format!("  let counter_{i} = {i};\n"));
    }
    source.push_str("  yield counter_0;\n}\npub fnc main(): int { return 0; }\n");
    let err = atto_compiler::compile(&source).unwrap_err();
    assert_eq!(err.code, "E_UNSUPPORTED");
    assert!(err.message.contains("16 live integer locals"), "{err}");
}


#[test]
fn loop_carried_values_remain_live_at_suspension() {
    let typed = program(RUNNING_TOTALS);
    let generator = &typed.generators[0];
    assert_eq!(generator.locals, 2);
    assert_eq!(generator.live_after.len(), generator.steps.len());
    let suspend = generator.steps.iter().position(|s|
        matches!(s, TGenStep::Yield(_))).expect("one yield");
    assert_eq!(generator.live_after[suspend], 0b11,
        "both count and total must survive the while-loop backedge");
}

#[test]
fn dead_scalar_slots_are_not_kept_live_after_yield() {
    let typed = program(r#"
        fnc *gen selective(): Generator<int> {
          let first = 7;
          let second = 11;
          yield first;
          yield second;
        }
        pub fnc main(): int {
          let sum = 0;
          for n in selective() do sum += n; done
          return sum;
        }
    "#);
    let gen = &typed.generators[0];
    assert_eq!(gen.locals, 2);
    let yields: Vec<_> = gen.steps.iter().enumerate()
        .filter(|(_, s)| matches!(s, TGenStep::Yield(_)))
        .map(|(i, _)| gen.live_after[i]).collect();
    assert_eq!(yields, vec![0b10, 0],
        "first value is dead after first yield, second survives until consumed");
}

#[test]
fn generator_with_no_frame_locals_has_empty_liveness_masks() {
    let typed = program(r#"
        fnc *gen values(): Generator<int> {
          yield 7;
          yield 11;
        }
        pub fnc main(): int { return 0; }
    "#);
    let gen = &typed.generators[0];
    assert_eq!(gen.locals, 0);
    assert!(gen.live_after.iter().all(|mask| *mask == 0));
}
