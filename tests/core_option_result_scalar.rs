//! Exact real Wasm guest tests. No host-model fallback, no execution skipping.
//! Local machine and CI must have node; a missing runtime fails the test.
//! The compiler-owned generic core library is a separate admission milestone.
use atto_compiler::compile;
use std::io::Write;
use std::process::{Command, Stdio};

const WASM_HEADER: &[u8] = b"\0asm\x01\0\0\0";

fn execute_guest(bytes: &[u8], expected: i64) {
    assert!(bytes.starts_with(WASM_HEADER), "compiler did not emit Wasm");
    // Only WebAssembly executes the returned guest instructions. No JS
    // simulation of Option/Result is permitted. Any guest stdio import fails.
    let script = r#"
      const fs = require('node:fs');
      (async () => {
        const wasm = fs.readFileSync(0);
        const noHost = () => { throw Error('unexpected host import'); };
        const { instance } = await WebAssembly.instantiate(wasm, {
          atto: {
            print_i64: noHost, print_bool: noHost, print_str: noHost,
            print_newline: noHost, actor_spawn: noHost, actor_send: noHost
          }
        });
        if (typeof instance.exports.main !== 'function') throw Error('missing guest main');
        const actual = instance.exports.main();
        const expected = BigInt(process.argv[1]);
        if (actual !== expected) throw Error('guest main mismatch: ' + actual + ' vs ' + expected);
        if (instance.exports.panic_code && instance.exports.panic_code.value !== 0)
          throw Error('guest panic code: ' + instance.exports.panic_code.value);
      })().catch(error => { console.error(error); process.exitCode = 1; });
    "#;
    let mut child = Command::new("node")
        .arg("-e").arg(script).arg(expected.to_string())
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
fn borrowed_option_result_tags_execute_wasm_and_preserve_copy_owners() {
    // This must compile through the real Rust frontend and execute Wasm.
    // A pure Rust host model or an unsupported-grammar rejection is not a pass.
    let source = include_str!("core_option_result/borrowed_read_42.atto");
    let first = compile(source).expect("borrowed Option/Result tag queries need typed admission");
    assert_eq!(first, compile(source).unwrap(), "borrow guest Wasm must be deterministic");
    execute_guest(&first, 42);
}

#[test]
fn borrowed_affine_payload_escape_requires_ownership_diagnostic() {
    let source = include_str!("core_option_result/negative_affine_borrow_escape.atto");
    let error = compile(source).expect_err("a borrowed affine Option must not return its owned payload");
    // A parser not recognizing borrow or class constructs is NOT a proof.
    // The compiler must reject the typed ownership violation itself.
    assert!(
        matches!(error.code, "E_OWNERSHIP" | "E_BORROW" | "E_MOVED"),
        "must reject on typed ownership grounds, not a syntax/type fallback: {error}"
    );
    assert!(error.pos.line > 0 && error.pos.col > 0, "{error}");
}
