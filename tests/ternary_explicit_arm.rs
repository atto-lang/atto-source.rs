use atto_compiler::compile;
use std::io::Write;
use std::process::{Command, Stdio};

// The browser compiler CI already requires Node. A missing engine is an
// explicit test failure, not a silent skip or a magic-byte-only pass.
fn assert_guest_returns_42(source: &str) {
    let wasm = compile(source).unwrap_or_else(|e| panic!("{source}: {e:?}"));
    assert_eq!(&wasm[..4], b"\0asm");
    let mut child = Command::new("node")
        .args(["-e", r#"
const assert = require('node:assert/strict');
const fs = require('node:fs');
(async () => {
  const bytes = fs.readFileSync(0);
  assert.ok(WebAssembly.validate(bytes), 'invalid guest Wasm');
  const {instance} = await WebAssembly.instantiate(bytes, {atto: {}});
  assert.equal(typeof instance.exports.main, 'function');
  assert.equal(instance.exports.main(), 42n);
  if (instance.exports.panic_code)
    assert.equal(instance.exports.panic_code.value, 0);
})().catch(err => { console.error(err); process.exitCode = 1; });
"#])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Node is required for actual Wasm guest execution");
    child.stdin.take().expect("guest stdin").write_all(&wasm)
        .expect("write compiled Wasm to engine");
    let output = child.wait_with_output().expect("wait for guest engine");
    assert!(
        output.status.success(),
        "{source}: guest execution failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn explicit_arm_results_execute_guest_wasm() {
    for source in [
        "pub fnc main(): int { return true ? { return 42; } : { ^ 7; }; }",
        "pub fnc main(): int { return false ? 7 : { ^ 42; }; }",
        "pub fnc main(): int { return true ? { return false ? 7 : 42; } : 0; }",
        "pub fnc main(): int { val int answer = true ? { return 40; } : { ^ 7; }; return answer + 2; }",
        "pub fnc main(): int { val int x = 9223372036854775807; return true ? { ^ 42; } : { return x + 1; }; }",
        "pub fnc main(): int { val int x = 9223372036854775807; return false ? { return x + 1; } : { ^ 42; }; }",
    ] {
        assert_guest_returns_42(source);
    }
}

#[test]
fn direct_brace_arms_fail_closed_without_explicit_result() {
    for expr in [
        "true ? {} : 7",
        "false ? 7 : {}",
        "true ? { status: 42 } : 7",
        "true ? { 42 } : 7",
        "true ? { val x = 42; return x; } : 7",
        "true ? { return 42; return 7; } : 0",
        "false ? 7 : { ^ 42; ^ 0; }",
    ] {
        let source = format!("pub fnc main(): int {{ return {expr}; }}");
        let error = compile(&source).expect_err(&source);
        assert!(format!("{error:?}").contains("E_VALUE_ARM"), "{source}: {error:?}");
    }
}
