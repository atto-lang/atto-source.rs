//! Restricted scalar receiver preview: typed source -> real Wasm bytes.
use atto_compiler::target_scalar::compile_scalar_receiver_preview;
use atto_compiler::lexer::lex;
use atto_compiler::target_syntax::parse_target_declarations;
use atto_compiler::target_typed::lower_target_signatures;

const SRC:&str=r#"
use namespace workers;
declare actor Worker;
mtd (Worker self) double(int n): int { return n * 2; }
mtd (Worker self) add(int n, int b): int { return n + b; }
mtd (Worker self) subtract(int n): int { return n - 1; }
mtd (Worker self) minus(int n): int { return -n; }
mtd (Worker self) minimum(): int { return -9223372036854775808; }
"#;

#[test]
fn scalar_preview_produces_stable_real_wasm_and_keeps_normal_compiler_closed() {
    let bytes=compile_scalar_receiver_preview(SRC).unwrap();
    assert_eq!(&bytes[..8],b"\0asm\x01\0\0\0");
    assert_eq!(bytes,compile_scalar_receiver_preview(SRC).unwrap());
    assert!(bytes.len()<4096);
    assert_eq!(atto_compiler::compile(SRC).unwrap_err().code,"E_UNSUPPORTED");
    let typed=lower_target_signatures(&parse_target_declarations(&lex(SRC).unwrap()).unwrap()).unwrap();
    assert!(typed.receivers.iter().all(|r|!r.body_lowered));
    assert!(typed.receiver_cleanup.iter().all(|p|p.leases.is_empty()));
}

#[test]
fn scalar_preview_rejects_actor_state_and_unproved_control_flow() {
    for source in [
        "declare actor Counter; define actor Counter as let int count = 0; end
          mtd (mut self: Counter) increment(int n): void {
              self.handle_state(critical-write |s| -> { s.count = s.count + n; });
          }",
        "declare actor A; mtd (A self) choose(int n): int {
            if n == 1 { return 1; } else { return 2; }
         }",
        "declare actor A; mtd (A self) escape(int n): int {
            return n; return n;
         }",
        "declare actor A; mtd (A self) divide(int n): int { return n / 2; }",
        "declare actor A; mtd (A self) take(int n): int { return n + missing; }",
        "declare actor A; mtd (A self) extra(int n): int { return n; n += 2; }",
        "declare actor A; mtd (A self) bool_result(bool n): bool { return n; }",
        "declare actor A; mtd (mut self: A) mutate(int n): int { return n; }",
        "declare actor A; mtd (A self) f(): int { return 9223372036854775808; }",
        "declare actor A; mtd (A self) f(): int { return true; }",
        "declare actor A; mtd (A self) f(int n): int { return n(n); }",
        "declare actor A; mtd (A self) f(int n): int { return (n)(); }",
        "declare actor A; mtd (A self) call(int n): int { return self.other(); }",
        "declare actor A; mtd (A self) f(int n): int { return (n + 2; }",
        "declare actor A; pub fnc main(): int { return 1; }
          mtd (A self) f(int n): int { return n; }",
    ] {
        assert!(compile_scalar_receiver_preview(source).is_err(),
            "unexpected scalar preview admission: {source}");
    }
}

#[test]
fn scalar_preview_accepts_nested_pure_checked_math_and_boundary_literals() {
    for source in [
        "declare actor A; mtd (A self) f(int n): int { return (n + 2) * (n - 3); }",
        "declare actor A; mtd (A self) f(): int { return -9223372036854775808; }",
        "declare actor A; mtd (A self) f(int n): int { return -(-n); }",
        "declare actor A; mtd (A self) f(): int { return 9223372036854775807; }",
    ] {
        let result=compile_scalar_receiver_preview(source);
        assert!(result.is_ok(),
            "valid subset not accepted: {source}; diagnostic: {:?}",result.err());
    }
}

#[test]
fn scalar_preview_enforces_depth_and_source_budgets() {
    let expression=format!("{}n{}","(".repeat(40),")".repeat(40));
    let nested=format!("declare actor A; mtd (A self) f(int n): int {{ return {expression}; }}");
    let deep_error=compile_scalar_receiver_preview(&nested).unwrap_err();
    assert_eq!(deep_error.code,"E_NESTING","full diagnostic: {deep_error:?}");
    let huge=format!("declare actor A; mtd (A self) f(int n): int {{ return n; }}\n{}",
        " ".repeat(atto_compiler::SOURCE_CAP));
    assert_eq!(compile_scalar_receiver_preview(&huge).unwrap_err().code,"E_SOURCE_LIMIT");
}
