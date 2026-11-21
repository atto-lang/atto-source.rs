//! Compiler-owned parsing, typed admission and genuine Wasm emission gates.
//! These tests do not prove guest runtime behavior; Node/Chromium execution
//! of exactly the same fixtures is required before advertising support.
const POSITIVES: &[(&str, &str)] = &[
    ("fair", include_str!("fixtures/pass/230-v2-static-select-fair.atto")),
    ("dynamic", include_str!("fixtures/pass/231-v2-dynamic-select-mutate.atto")),
    ("nb", include_str!("fixtures/pass/232-v2-nb-select-snapshot.atto")),
    ("try-miss", include_str!("fixtures/pass/233-v2-try-select-empty.atto")),
    ("one-winner", include_str!("fixtures/pass/234-v2-select-single-winner.atto")),
    ("blocking-generation", include_str!("fixtures/pass/235-v2-blocking-select-generation.atto")),
    ("closed-loser", include_str!("fixtures/pass/236-v2-select-closed-loser.atto")),
];

#[test]
fn select_plans_produce_real_binary_wasm_with_no_script_fallback() {
    for (name, source) in POSITIVES {
        let bytes = atto_compiler::compile(source)
            .unwrap_or_else(|e| panic!("{name} failed source compilation: {e}"));
        assert!(bytes.starts_with(b"\0asm\x01\0\0\0"), "{name} did not emit Wasm");
        assert!(bytes.len() > 100, "{name} empty binary guest");
    }
}

#[test]
fn select_plans_reject_unsafe_or_unimplemented_forms_without_emitting_guest() {
    let seed = "pub suspends fnc main(): int { val c = Channel<int>.bounded(1); ";
    let tests = [
        (format!("{seed} val p = SelectPlan.from_list([]); return 42; }}"), "E_TYPE"),
        (format!("{seed} val p = SelectPlan.from_list([1]); return 42; }}"), "E_TYPE"),
        (format!("{seed} val c2 = Channel<bool>.bounded(1); val p = SelectPlan.from_list([SelectCase.read(c2)]); return 42; }}"), "E_TYPE"),
        (format!("{seed} val p = SelectPlan.from_map([SelectCase.read(c)]); return 42; }}"), "E_UNSUPPORTED"),
        (format!("{seed} val p = DynamicSelectPlan<string>.from_list([SelectCase.read(c)]); return 42; }}"), "E_UNSUPPORTED"),
        (format!("{seed} val p = SelectPlan.from_list([SelectCase.read(c)]); p.put(0, SelectCase.read(c)); return 42; }}"), "E_TYPE"),
        (format!("{seed} val p = SelectPlan.from_list([SelectCase.read(c)]); val v = try select fair from 42; return 42; }}"), "E_TYPE"),
    ];
    for (source, code) in tests {
        let err = atto_compiler::compile(&source).expect_err("unsupported select must fail");
        assert_eq!(err.code, code, "{err}");
    }
}

#[test]
fn standalone_plan_select_statements_compile_but_discarding_nb_is_denied() {
    let prefix = r#"pub suspends fnc main(): int {
        val ch = Channel<int>.bounded(1);
        val plan = SelectPlan.from_list([SelectCase.read(ch)]);
        writech ch, 42;
    "#;
    for statement in ["select fair from plan;", "try select first from plan;"] {
        let source = format!("{prefix} {statement} return 42; }}");
        let guest = atto_compiler::compile(&source)
            .unwrap_or_else(|e| panic!("{statement}: {e}"));
        assert!(guest.starts_with(b"\0asm\x01\0\0\0"));
    }
    for (statement, code) in [
        ("nb select fair from plan;", "E_UNSUPPORTED"),
        ("select { on readch(ch) as value { return value; } }", "E_UNSUPPORTED"),
    ] {
        let source = format!("{prefix} {statement} return 42; }}");
        let diagnostic = atto_compiler::compile(&source)
            .expect_err("an unverified select form must not emit an executable guest");
        assert_eq!(diagnostic.code, code, "{statement}: {diagnostic}");
    }
}

#[test]
fn nonblocking_select_gets_a_guest_scheduler_even_when_main_is_not_suspends() {
    // An unrelated suspends declaration explicitly opts in to v2, while the
    // sync main creates a task through __plan_nb. Entry selection happens
    // before the runtime prelude is lowered: this is a regression for a
    // hidden task whose scheduler dependency was previously missed.
    let source = r#"
suspends fnc opt_in(): int { return 1; }
pub fnc main(): int {
  val ch = Channel<int>.bounded(1);
  val plan = SelectPlan.from_list([SelectCase.read(ch)]);
  val Future<Option<SelectResult>> pending = nb select first from plan;
  // A synchronous callable cannot use suspending writech. This test
  // inspects the compiled entry wrapper, not the lifetime of an awaited plan.
  return 42;
}
"#;
    let p = atto_compiler::v2::parse::Parser::new(
        atto_compiler::lexer::lex(source).expect("source lexer")
    ).parse_program().expect("source parser");
    let runtime = include_str!("../src/v2/prelude.atto");
    let prelude = atto_compiler::v2::parse::Parser::new(
        atto_compiler::lexer::lex(runtime).expect("runtime lexer")
    ).parse_program().expect("runtime parser");
    let module = atto_compiler::v2::check::check(&p, &prelude)
        .expect("v2 lowering must admit bounded plan");
    assert_eq!(module.fns[module.main as usize].name, "atto.start",
        "a nested __plan_nb task must trigger the scheduler at entry selection");
    let wasm = atto_compiler::compile(source).expect("real Wasm compilation");
    assert!(wasm.starts_with(b"\0asm\x01\0\0\0"));
}
