//! Compiler-level test: real guest Wasm output, not a preview parser acceptance.
//! Execution/lifecycle remain unproven until Chromium runs at this exact SHA.

const EXACT: &str = include_str!("fixtures/pass/205-play-scoped-span.atto");

#[test]
fn exact_source_emits_webassembly() {
    let bytes = atto_compiler::compile(EXACT).expect("exact scoped span demo must compile");
    assert!(bytes.starts_with(b"\0asm\x01\0\0\0"), "must emit WebAssembly binary");
}

#[test]
fn aliased_imports_link_to_the_same_runtime_exports() {
    let source = r#"
import span as start_span from pkg "core.telemetry";
import sleep as delay from pkg "std.time";
pub suspends fnc main(): int {
  using start_span("alias") as s {
    with timeout(2000) {
      await delay(1);
      s.set_attribute("after", "resume");
    }
  }
  return 0;
}"#;
    assert!(atto_compiler::compile(source).is_ok());
}

#[test]
fn invalid_named_imports_and_suspend_effects_fail_closed() {
    for source in [
        r#"import span from pkg "other.module"; pub suspends fnc main(): int { return 0; }"#,
        r#"import bogus from pkg "core.telemetry"; pub suspends fnc main(): int { return 0; }"#,
        r#"import span, span from pkg "core.telemetry"; pub suspends fnc main(): int { return 0; }"#,
        r#"import span as stdio from pkg "core.telemetry"; pub suspends fnc main(): int { return 0; }"#,
        r#"import sleep from pkg "std.time"; pub suspends fnc main(): int { sleep(1); return 0; }"#,
        r#"import sleep from pkg "std.time"; pub fnc main(): int { await sleep(1); return 0; }"#,
    ] {
        assert!(atto_compiler::compile(source).is_err(), "must reject {source:?}");
    }
}
