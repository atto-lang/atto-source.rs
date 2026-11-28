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
fn invalid_named_imports_rejected_inside_the_actual_scoped_profile() {
    for (source, expected) in [
        (EXACT.replace("pkg \"core.telemetry\"", "pkg \"other.module\""), "E_MODULE"),
        (EXACT.replace("import span from", "import bogus from"), "E_MODULE"),
        (EXACT.replace("import span from", "import span, span from"), "E_DUPLICATE"),
        (EXACT.replace("import span from", "import span as stdio from"), "E_DUPLICATE"),
    ] {
        let error = atto_compiler::compile(&source)
            .expect_err("bad import must not emit guest Wasm");
        assert_eq!(error.code, expected, "unexpected diagnostic for {source}");
    }
}

#[test]
fn scoped_profile_does_not_bypass_legacy_guards_or_effect_checks() {
    for source in [
        EXACT.replace("await sleep(1);", "sleep(1); await sleep(1);"),
        EXACT.replace("pub suspends fnc main", "pub fnc main"),
        EXACT.replace("await sleep(1);", "task { return 42; }; await sleep(1);"),
        EXACT.replace("await sleep(1);", "spawn Worker(); await sleep(1);"),
    ] {
        assert!(atto_compiler::compile(&source).is_err(),
            "unsafe or missing-effect syntax must never emit guest Wasm: {source}");
    }
    let normal = "pub fnc main(): int { return 6 * 7; }";
    assert!(atto_compiler::compile(normal).is_ok(),
        "legacy scalar compilation must be preserved");
}
