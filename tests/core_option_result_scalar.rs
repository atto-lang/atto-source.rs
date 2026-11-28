//! Exact real Wasm guest tests. No host-model fallback, no execution skipping.
//! Local machine and CI must have node; a missing runtime fails the test.
//! The compiler-owned generic core library is a separate admission milestone.
use atto_compiler::compile;
use std::io::Write;
use std::process::{Command, Stdio};

const WASM_HEADER: &[u8] = b"\0asm\x01\0\0\0";

fn execute_guest(bytes: &[u8], expected: i64) {
    execute_guest_with_trace(bytes, expected, "");
}

fn execute_guest_with_trace(bytes: &[u8], expected: i64, trace: &str) {
    assert!(bytes.starts_with(WASM_HEADER), "compiler did not emit Wasm");
    // Only WebAssembly executes the returned guest instructions. No JS
    // simulation of Option/Result is permitted. Only the exact expected integer output is allowed.
    let script = r#"
      const fs = require('node:fs');
      (async () => {
        const wasm = fs.readFileSync(0);
        const noHost = () => { throw Error('unexpected host import'); };
        let trace = '';
        const { instance } = await WebAssembly.instantiate(wasm, {
          atto: {
            print_i64: n => { trace += String(n); }, print_bool: noHost, print_str: noHost,
            print_newline: () => { trace += '\n'; }, actor_spawn: noHost, actor_send: noHost
          }
        });
        if (typeof instance.exports.main !== 'function') throw Error('missing guest main');
        const actual = instance.exports.main();
        const expected = BigInt(process.argv[1]);
        if (actual !== expected) throw Error('guest main mismatch: ' + actual + ' vs ' + expected);
        if (trace !== process.argv[2]) throw Error('unexpected guest trace: ' + JSON.stringify(trace));
        if (instance.exports.panic_code && instance.exports.panic_code.value !== 0)
          throw Error('guest panic code: ' + instance.exports.panic_code.value);
      })().catch(error => { console.error(error); process.exitCode = 1; });
    "#;
    let mut child = Command::new("node")
        .arg("-e").arg(script).arg(expected.to_string()).arg(trace)
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().expect("Node.js is required for real WebAssembly guest execution");
    child.stdin.take().expect("piped guest stdin")
        .write_all(bytes).expect("write exact Wasm artifact to Node");
    let output = child.wait_with_output().expect("run real Wasm guest");
    assert!(output.status.success(), "guest: {}", String::from_utf8_lossy(&output.stderr));
    assert!(output.stdout.is_empty(), "unexpected stdout: {:?}", output.stdout);
    assert!(output.stderr.is_empty(), "unexpected stderr: {:?}", output.stderr);
}

#[test]
fn nested_sum_guest_executes_to_42() {
    let source = include_str!("core_option_result/positive_42.atto");
    let first = compile(source).expect("source compiler must admit correct Option/Result");
    assert_eq!(first, compile(source).unwrap(), "non-deterministic emitted Wasm");
    execute_guest(&first, 42);
}

#[test]
fn tag_zero_guest_executes_to_42() {
    let source = include_str!("core_option_result/tag_zero_42.atto");
    let bytes = compile(source).expect("zero-valued Some/Ok must be admitted");
    assert_eq!(bytes, compile(source).unwrap(), "non-deterministic emitted Wasm");
    execute_guest(&bytes, 42);
}

#[test]
fn wrong_nominal_carrier_does_not_emit_wasm() {
    let source = include_str!("core_option_result/negative_wrong_carrier.atto");
    let error = compile(source).expect_err("Err cannot construct Option<int>");
    assert_eq!(error.code, "E_TYPE", "{error}");
    assert!(error.pos.line > 0 && error.pos.col > 0, "{error}");
}

#[test]
fn nonexhaustive_match_does_not_emit_wasm() {
    let source = include_str!("core_option_result/negative_nonexhaustive.atto");
    let error = compile(source).expect_err("incomplete Option match cannot compile");
    assert_eq!(error.code, "E_MATCH", "must reject non-exhaustive Option match: {error}");
    assert!(error.pos.line > 0 && error.pos.col > 0, "{error}");
}

#[test]
fn explicit_borrow_marker_on_copyable_option_result_is_nondestructive() {
    // These concrete carriers are Copy; this checks nondestructive value-copy
    // behavior after borrow! syntax, NOT a lifetime-bearing generic loan.
    // A pure Rust host model is not a compiled guest pass.
    let source = include_str!("core_option_result/borrowed_read_42.atto");
    let first = compile(source).expect("borrowed Option/Result tag queries need typed admission");
    assert_eq!(first, compile(source).unwrap(), "borrow guest Wasm must be deterministic");
    execute_guest(&first, 42);
}

#[test]
fn borrowed_affine_payload_escape_requires_ownership_diagnostic() {
    let source = include_str!("core_option_result/negative_affine_borrow_escape.atto");
    let error = compile(source).expect_err("a borrowed affine class cannot become an owned return");
    // This class-based test reaches the existing typed ownership checker.
    // An E_UNSUPPORTED_TYPE on Option<Payload> would not be evidence.
    assert_eq!(error.code, "E_BORROW", "expected the semantic loan escape diagnostic: {error}");
    assert!(error.pos.line > 0 && error.pos.col > 0, "{error}");
}

#[test]
fn unproved_affine_option_payload_remains_unsupported() {
    let source = include_str!("core_option_result/negative_unproved_affine_option.atto");
    let error = compile(source).expect_err("Option<Payload> must not invent an affine Wasm ABI");
    assert_eq!(error.code, "E_UNSUPPORTED_TYPE", "{error}");
    assert!(error.pos.line > 0 && error.pos.col > 0, "{error}");
}

#[test]
fn borrowed_concrete_adt_temporary_and_mutable_loan_are_denied() {
    for source in [
        include_str!("core_option_result/negative_borrow_option_temporary.atto"),
        include_str!("core_option_result/negative_borrow_mut_option.atto"),
        include_str!("core_option_result/negative_borrow_option_return.atto"),
    ] {
        let error = compile(source).expect_err(&format!("unproven ADT loan must not emit Wasm: {source}"));
        assert_eq!(error.code, "E_BORROW", "{error}");
        assert!(error.pos.line > 0 && error.pos.col > 0, "{error}");
    }
}

#[test]
fn conditional_return_executes_real_guest() {
    let cases = [
        ("pub fnc main(): int { ^? 0; ^? -42; return 1; }", -42),
        ("pub fnc main(): int { return? 0; return? 40 + 2; return 1; }", 42),
        ("pub fnc main(): int { ^?=0 42; ^?=0 0; return 1; }", 0),
        ("pub fnc main(): int { ^?=[1,42] 7; return?=[1,42] 42; return 1; }", 42),
        ("pub fnc main(): int { ^?=[-9223372036854775808,42] -9223372036854775808; return 1; }", i64::MIN),
        ("fnc choose(bool x): bool { ^? x; return false; } pub fnc main(): int { if choose(true) then return 42; fi return 1; }", 42),
        ("fnc choose(bool x): bool { ^? x; return true; } pub fnc main(): int { if choose(false) then return 42; fi return 1; }", 42),
        ("fnc choose(bool x): bool { ^?=false x; return true; } pub fnc main(): int { if choose(false) then return 1; fi return 42; }", 42),
        ("fnc get(): Option<int> { return Some(0); } pub fnc main(): int { ^? get(); return 42; }", 0),
        ("fnc get(): Option<int> { return None; } pub fnc main(): int { ^? get(); return 42; }", 42),
        ("fnc get(): Result<int,string> { return Ok(0); } pub fnc main(): int { ^? get(); return 42; }", 0),
        ("fnc get(): Result<int,string> { return Err(\"no\"); } pub fnc main(): int { ^? get(); return 42; }", 42),
    ];
    for (source, expected) in cases {
        let bytes = compile(source).unwrap_or_else(|e| panic!("{source}: {e}"));
        assert_eq!(bytes, compile(source).unwrap());
        execute_guest(&bytes, expected);
    }
}

#[test]
fn conditional_return_checks_operand_once_and_only_selected_return_runs() {
    let source = "fnc attempt(int n): int { stdio.println(n); return n; } pub fnc main(): int { ^?=[1,42] attempt(7); ^?=[1,42] attempt(42); ^? attempt(9); return 1; }";
    let bytes = compile(source).unwrap();
    execute_guest_with_trace(&bytes, 42, "7\n42\n");
}

#[test]
fn conditional_return_rejects_invalid_programs_without_emitting_guest() {
    let cases = [
        "pub fnc main(): int { ^? true; return 42; }",
        "pub fnc main(): int { ^? 0; }",
        "pub fnc main(): int { ^? \"yes\"; return 42; }",
        "pub fnc main(): int { ^?=[true] 42; return 1; }",
        "pub fnc main(): int { ^?=[true,42] 42; return 1; }",
        "pub fnc main(): int { ^?=[] 42; return 1; }",
        "pub fnc main(): int { ^?=[42,] 42; return 1; }",
        "pub fnc main(): int { ^?=[42,42] 42; return 1; }",
        "pub fnc main(): int { ^ ? 42; return 1; }",
        "pub fnc main(): int { return ? 42; return 1; }",
        "pub fnc main(): int { ^? =42 42; return 1; }",
        "pub fnc main(): int { val x = ^? 42; return x; }",
        "pub fnc main(): int { ^?; return 42; }",
        "pub fnc main(): int { ^? match true over on true -> 42, on false -> 0 end; return 1; }",
    ];
    for source in cases {
        let error = compile(source).expect_err(source);
        assert!(error.pos.line > 0 && error.pos.col > 0, "{source}: {error}");
    }
}
