//! Exact syntax fixture for /play's resource scope preview.
//! Since the experimental v2 async backend landed, this source compiles: v2
//! lowers using/with timeout/await to resumable guest state machines, and
//! core.telemetry/std.time resolve to in-guest prelude runtime exports (not a
//! general package linker). Execution and exactly-once span cleanup are checked
//! in a real Chromium Worker by tests/browser-e2e.mjs (fixtures 205/206 and
//! the timeout-unwind case).
use atto_compiler::lexer::lex;
use atto_compiler::target_syntax::parse_target_declarations;

const PREVIEW: &str = r#"
import span from pkg "core.telemetry";
import sleep from pkg "std.time";

pub suspends fnc main(): int {
  using span("playground.demo") as current_span {
    with timeout(2000) {
      current_span.set_attribute("feature", "import");
      await sleep(1);
      stdio.println("span retained over suspension");
    }
  }
  return 0;
}
"#;

#[test]
fn named_imports_of_scoped_resource_example_are_supported_by_target_headers() {
    let source = r#"
        import span from pkg "core.telemetry";
        import sleep from pkg "std.time";
        pub fnc main(): int { return 0; }
    "#;
    let headers = parse_target_declarations(&lex(source).expect("lex")).expect("headers");
    assert_eq!(headers.imports, ["span", "sleep"]);
}

#[test]
fn exact_scoped_resource_demo_compiles_to_a_guest() {
    let guest = atto_compiler::compile(PREVIEW).expect("real executable guest required");
    assert!(guest.starts_with(b"\0asm"), "must emit WebAssembly bytes");
}
