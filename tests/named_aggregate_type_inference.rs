use atto_compiler::compile;

const POSITIVE:&str=r#"
pub fnc main(): int {
    const Record<infer T1> r=(a=true,b="xyz",c=5);
    const Tuple<infer T2> t=(true,"xyz",5);
    const struct<infer T3> s={a:true,b:"xyz",c:5};
    const Record<T1> r2=(c=10,b="abc",a=false);
    const Tuple<T2> t2=(false,"abc",12);
    const struct<T3> s2={c:20,b:"abc",a:false};
    return r2.c+t2[2]+s2.c;
}
"#;

#[test]
fn named_scalar_aggregate_reuse_emits_checked_guest() {
    let bytes=compile(POSITIVE).expect("named aggregate scalar guest");
    assert!(bytes.starts_with(b"\0asm\x01\0\0\0"));
    assert_eq!(bytes,compile(POSITIVE).unwrap());
    assert_guest_result(&bytes, 42);
    // The second literal's values must be observed, not the exemplars.
    let different = POSITIVE.replace("c=10", "c=11")
        .replace("false,\"abc\",12", "false,\"abc\",14")
        .replace("c:20", "c:23");
    let changed = compile(&different).expect("different aggregate values");
    assert_guest_result(&changed, 48);
}
#[test]
fn malformed_or_mismatched_shapes_emit_no_guest() {
    for (body,code) in [
      ("const Record<infer> r=(a=1);","E_TYPE_NAME"),
      ("const Record<infer T> r=(a=1); const Record<U> s=(a=1);","E_UNKNOWN_TYPE"),
      ("const Record<infer T> r=(a=1); const Tuple<infer T> s=(1,2);","E_DUPLICATE_TYPE"),
      ("const Record<infer T> r=(a=1,a=2);","E_DUPLICATE_FIELD"),
      ("const Record<infer T> r=(a=1); const Record<T> s=(a=true);","E_TYPE_MISMATCH"),
      ("const Record<infer T> r=(a=1); const Record<T> s=(a=1,b=2);","E_TYPE_MISMATCH"),
      ("const Record<infer T> r=(a=1,b=2); const Record<T> s=(a=1);","E_TYPE_MISMATCH"),
      ("const Tuple<infer T> r=(1,true); const Tuple<T> s=(true,1);","E_TYPE_MISMATCH"),
      ("const Tuple<infer T> r=(1,true); const Tuple<T> s=(1);","E_TYPE_MISMATCH"),
      ("const struct<infer T> r={a:1}; const Record<T> s=(a=1);","E_AGGREGATE_KIND"),
      ("const Record<infer T> r=(a=unknown);","E_TYPE_INFER_AMBIGUOUS"),
    ] {
        let source=format!("pub fnc main(): int {{ {body} return 0; }}");
        assert_eq!(compile(&source).unwrap_err().code,code,"{source}");
    }
}
#[test]
fn no_whole_aggregate_or_runtime_identity_escape() {
    for result in ["r","r.missing","r.a + unknown"] {
        let source=format!("pub fnc main(): int {{ const Record<infer T> r=(a=1); return {result}; }}");
        assert!(compile(&source).is_err(),"{source}");
    }
}

// Execute the actual emitted bytes in the engine already used by browser CI.
// No mock compiler, skipped engine, or magic-header-only success.
fn assert_guest_result(bytes: &[u8], expected: i64) {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new("node")
        .args(["-e", r#"
const assert = require('node:assert/strict');
const fs = require('node:fs');
(async () => {
  const bytes = fs.readFileSync(0);
  assert.ok(WebAssembly.validate(bytes));
  const {instance} = await WebAssembly.instantiate(bytes, {atto: {}});
  assert.equal(typeof instance.exports.main, 'function');
  assert.equal(instance.exports.main(), BigInt(process.argv[1]));
  if (instance.exports.panic_code)
    assert.equal(instance.exports.panic_code.value, 0);
})().catch(err => { console.error(err); process.exitCode = 1; });
"#])
        .arg(expected.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Node is required for actual Wasm execution");
    child.stdin.take().expect("guest stdin").write_all(bytes)
        .expect("send Wasm bytes to engine");
    let output = child.wait_with_output().expect("wait for Wasm engine");
    assert!(output.status.success(), "guest execution failed: {}",
        String::from_utf8_lossy(&output.stderr));
}

#[test]
fn named_aggregate_erasure_does_not_bypass_reserved_names() {
    for alias in ["if", "while", "select", "boolean", "String", "Pointer",
                  "ActorRef", "AtomicInt", "i64", "ReadLoan"] {
        let source = format!("pub fnc main(): int {{ const Record<infer {alias}> r=(a=1); return r.a; }}");
        assert_eq!(compile(&source).expect_err(&source).code, "E_TYPE_NAME", "{source}");
    }
    for source in [
        "pub fnc main(): int { const Record<infer R> if=(a=1); return 0; }",
        "pub fnc main(): int { const Record<infer R> r=(while=1); return 0; }",
    ] {
        assert_eq!(compile(source).expect_err(source).code, "E_TYPE_NAME", "{source}");
    }
    // Reserved checks are case sensitive, not a lowercasing normalization.
    let source = "pub fnc main(): int { const Record<infer If> r=(a=1); const Record<If> s=(a=42); return s.a; }";
    let bytes = compile(source).expect("distinct legal type name");
    assert_guest_result(&bytes, 42);
}
