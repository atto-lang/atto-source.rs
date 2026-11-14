//! Executable bounded global Symbol profile: no implicit dynamic interning.
use atto_compiler::compile;

fn pass(source: &str) -> Vec<u8> {
    compile(source).unwrap_or_else(|e| panic!("expected valid Symbol guest, got {e}"))
}
fn fail(source: &str, code: &str) {
    let error = compile(source).expect_err("unsafe Symbol program should not emit guest Wasm");
    assert_eq!(error.code, code, "{error}");
}
const HEADER: &str = "define pub symbol #ready;\ndefine pub symbol #stopped;\n";

#[test]
fn statically_declared_symbols_are_typed_and_deterministic() {
    let source = format!("{HEADER}pub fnc main(): int {{\n        const Symbol x = #ready;\n        const Symbol y = #stopped;\n        if x eq #ready then\n          if y neq x then return 42; fi\n        fi\n        return 0;\n        }}");
    let first = pass(&source);
    assert_eq!(first, pass(&source), "AOT Wasm binary must be bit-for-bit deterministic");
    assert_eq!(&first[..8], b"\0asm\x01\0\0\0");
}

#[test]
fn supervisor_and_actor_can_share_declared_symbol_in_typed_mail() {
    let source = format!("{HEADER}        define actor Worker as\n          receive(ActorMail<Symbol> mail): void {{\n            do match mail.value over\n              on #ready -> {{ stdio.println(7); }}\n              on #stopped -> {{ stdio.println(8); }}\n            end\n            return;\n          }}\n        end\n        pub fnc main(): int {{\n          const Symbol signal = #ready;\n          val worker = spawn Worker();\n          worker.send(signal);\n          return 0;\n        }}");
    pass(&source);
}

#[test]
fn never_conflate_symbol_string_or_integer() {
    fail("pub fnc main(): int { const Symbol s = #ready; return 0; }", "E_SYMBOL_UNBOUND");
    fail(&format!("{HEADER}pub fnc main(): int {{ if #ready eq 1 then return 1; fi return 0; }}"), "E_TYPE");
    fail(&format!("{HEADER}pub fnc main(): int {{ if #ready eq \"#ready\" then return 1; fi return 0; }}"), "E_TYPE");
}

#[test]
fn private_and_undeclared_symbols_fail_closed() {
    fail("define symbol #secret; pub fnc main(): int { return 0; }", "E_UNSUPPORTED");
    fail("define pub symbol #ready; define pub symbol #ready; pub fnc main(): int { return 0; }", "E_DUPLICATE");
    fail(&format!("{HEADER}pub fnc main(): int {{ val worker = spawn Worker(); worker.send(1); return 0; }}\n        define actor Worker as receive(ActorMail<Symbol> mail): void {{ return; }} end"), "E_TYPE");
}
