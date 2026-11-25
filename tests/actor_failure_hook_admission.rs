//! Pre-supervision safety: lifecycle callback names must never compile as
//! ordinary methods of the actor that just failed.
#[test]
fn actor_failure_hooks_fail_closed_without_supervisor_control() {
    for hook in ["on_panic", "on_raise", "on_throw"] {
        for form in [
            format!("{hook}(): void {{ return; }}"),
            format!("fnc {hook}(): void {{ return; }}"),
            format!("method {hook}(): void {{ return; }}"),
        ] {
            let source = format!(
                "define actor Child as\n  {form}\nend\npub fnc main(): int {{ return 0; }}\n"
            );
            let diagnostic = atto_compiler::compile(&source)
                .expect_err(&format!("actor hook {form} must not emit a Wasm guest"));
            assert_eq!(diagnostic.code, "E_UNSUPPORTED", "{form}: {diagnostic}");
            assert!(diagnostic.message.contains("supervisor CONTROL dispatch"),
                "{form}: {diagnostic}");
            assert_eq!(diagnostic.pos.line, 2, "{form}: {diagnostic}");
        }
    }
}
