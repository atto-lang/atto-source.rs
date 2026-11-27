use atto_compiler::compile;
use std::io::Write;
use std::process::{Command, Stdio};

fn execute(source: &str, oracle: &str) {
    let guest = compile(source).unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(guest, compile(source).expect("deterministic artifact"));
    let harness = format!(r#"
import assert from 'node:assert/strict';
import {{ readFileSync }} from 'node:fs';
import {{ createScopedTelemetry }} from './runtime/scoped-telemetry.mjs';

const telemetry = createScopedTelemetry();
const module = new WebAssembly.Module(readFileSync(0));
const instance = new WebAssembly.Instance(module, {{ atto: {{
  ...telemetry.imports,
  print_i64: () => {{}},
  print_newline: () => telemetry.setEnabled(false),
}} }});
telemetry.bindMemory(instance.exports.memory);
const run = () => telemetry.invoke(instance.exports.main);
{oracle}
"#);
    let mut child = Command::new("node")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args(["--input-type=module", "-e", &harness])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Node executes the compiler-emitted Wasm");
    child.stdin.take().unwrap().write_all(&guest).unwrap();
    let result = child.wait_with_output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
}

#[test]
fn nested_scope_closes_lifo_on_return_even_if_the_host_disables_recording() {
    execute(r#"
pub fnc main(): int {
  using telemetry.span("checkout") as outer {
    using telemetry.span("payment") as inner {
      stdio.println(0);
      return 42;
    }
  }
  return 1;
}
"#,
    r#"
assert.equal(run(), 42n);
assert.deepEqual(telemetry.drain(), {completed: [], active: 0, dropped: 0});
telemetry.setEnabled(true);
assert.equal(run(), 42n);
const receipt = telemetry.drain();
assert.deepEqual(receipt.completed.map(s => s.operation), ['payment', 'checkout']);
const [child, parent] = receipt.completed;
assert.equal(child.parentId, parent.id);
assert.equal(parent.parentId, 0n);
assert.notEqual(child.site, parent.site);
assert.equal(child.disposition, 'closed');
assert.equal(parent.disposition, 'closed');
assert.equal(receipt.active, 0);
assert.equal(receipt.dropped, 0);
assert.equal(run(), 42n);
assert.deepEqual(telemetry.drain(), {completed: [], active: 0, dropped: 0});
"#);
}

#[test]
fn fallthrough_and_guest_exception_use_the_same_scope_cleanup() {
    execute(r#"
fnc fail(): int {
  using telemetry.span("failure") as failed {
    throw Error("ordinary");
  }
  return 1;
}
pub fnc main(): int {
  using telemetry.span("fallthrough") as finished {}
  try { return fail(); }
  catch (Error e) { return 42; }
}
"#,
    r#"
telemetry.setEnabled(true);
assert.equal(run(), 42n);
const { completed, active, dropped } = telemetry.drain();
assert.deepEqual(completed.map(s => [s.operation, s.parentId, s.disposition]), [
  ['fallthrough', 0n, 'closed'], ['failure', 0n, 'closed'],
]);
assert.equal(active, 0);
assert.equal(dropped, 0);
"#);
}

#[test]
fn option_propagation_closes_the_scope_before_returning_none() {
    execute(r#"
fnc read(): Option<int> { return None; }
fnc compute(): Option<int> {
  using telemetry.span("propagation") as scope {
    val value: int = read()?;
    return Some(value + 1);
  }
  return Some(1);
}
pub fnc main(): int {
  match compute() over
    on Some(value) -> { return value; }
    on None -> { return 42; }
  end
}
"#,
    r#"
telemetry.setEnabled(true);
assert.equal(run(), 42n);
const { completed, active, dropped } = telemetry.drain();
assert.deepEqual(completed.map(s => [s.operation, s.disposition]), [['propagation', 'closed']]);
assert.equal(active, 0);
assert.equal(dropped, 0);
"#);
}

#[test]
fn fatal_wasm_trap_retires_host_scope_and_preserves_panic() {
    execute(r#"
fnc overflow(int input): int {
  using telemetry.span("overflow") as failed {
    return input + 1;
  }
  return 0;
}
pub fnc main(): int { return overflow(9223372036854775807); }
"#,
    r#"
telemetry.setEnabled(true);
assert.throws(run, WebAssembly.RuntimeError);
assert.equal(instance.exports.panic_code.value, 1);
const { completed, active, dropped } = telemetry.drain();
assert.deepEqual(completed.map(s => [s.operation, s.disposition]), [['overflow', 'abandoned']]);
assert.equal(active, 0);
assert.equal(dropped, 0);
"#);
}

#[test]
fn span_alias_cannot_escape_or_shadow_an_existing_binding() {
    for body in [
        r#"using telemetry.span("test") as scope { return scope; } return 0;"#,
        r#"using telemetry.span("test") as scope { val scope = 1; } return 0;"#,
        r#"val scope = 1; using telemetry.span("test") as scope {} return 0;"#,
        r#"using telemetry.span("test") as scope { scope.close_scope(); } return 0;"#,
    ] {
        let source = format!("pub fnc main(): int {{ {body} }}");
        let denied = compile(&source).expect_err("opaque scope handle");
        assert_eq!(denied.code, "E_TELEMETRY_ALIAS", "{denied}");
    }
}
