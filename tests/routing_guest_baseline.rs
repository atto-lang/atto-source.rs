//! Real compiled Atto scalar route probe; not production trie execution.
use atto_compiler::compile;
use std::io::Write;
use std::process::{Command, Stdio};

const POSITIVE: &str = include_str!("fixtures/pass/174-routing-scalar-wasm.atto");
const ARRAY_FAIL: &str = include_str!("fixtures/fail/174-routing-array-unlowered.atto");

#[test]
fn real_atto_scalar_route_wasm_guest_exits_42() {
    let wasm = compile(POSITIVE).unwrap_or_else(|d| panic!("Atto compile rejected guest: {d}"));
    assert!(wasm.starts_with(b"\0asm\x01\0\0\0"), "not a guest Wasm binary");
    assert_eq!(wasm, compile(POSITIVE).expect("deterministic compiler"));
    // Execute generated Wasm with a strict, no-effects host; do not substitute
    // a Rust router implementation for the compiled Atto guest.
    let script = r#"
const fs = require('node:fs');
const wasm = fs.readFileSync(0);
if (!WebAssembly.validate(wasm)) throw Error('invalid compiled Wasm');
const mod = new WebAssembly.Module(wasm);
const allowed = new Set(['print_i64', 'print_bool', 'print_str',
  'print_newline', 'print_f64', 'actor_spawn', 'actor_send',
  'state_register', 'state_enter', 'state_leave']);
for (const imp of WebAssembly.Module.imports(mod)) {
  if (imp.module !== 'atto' || !allowed.has(imp.name))
    throw Error('unexpected host import ' + imp.module + '.' + imp.name);
}
const deny = () => { throw Error('unexpected host effect'); };
const atto = Object.fromEntries([...allowed].map(name => [name, deny]));
const instance = new WebAssembly.Instance(mod, { atto });
const value = instance.exports.main();
if (value !== 42n) throw Error('guest result was ' + String(value));
if (instance.exports.panic_code.value !== 0) throw Error('guest panicked');
process.stdout.write('ATTO_SCALAR_ROUTE_GUEST_EXECUTED exit=42 panic=0\n');
"#;
    let mut child = Command::new("node").args(["-e", script])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().expect("Node is required for real Wasm guest execution");
    child.stdin.take().expect("child input").write_all(&wasm).expect("feed guest Wasm");
    let output = child.wait_with_output().expect("collect guest result");
    assert!(output.status.success(), "compiled guest failed: {}", String::from_utf8_lossy(&output.stderr));
    let actual = String::from_utf8(output.stdout).expect("guest output");
    assert_eq!(actual, "ATTO_SCALAR_ROUTE_GUEST_EXECUTED exit=42 panic=0\n");
    print!("{actual}");
}

#[test]
fn compiler_fails_closed_on_unlowered_general_arrays() {
    let d = compile(ARRAY_FAIL).expect_err("general Array values must be unsupported");
    assert_eq!(d.code, "E_UNSUPPORTED", "{d}");
    println!("ATTO_ROUTE_BLOCKER array={}", d.code);
}

#[test]
fn compiler_fails_closed_on_unsupported_string_replace() {
    let src = r#"pub fnc main(): int {
        val s = "aXb";
        stdio.println(s.replace_literal("X", "-"));
        return 0;
    }"#;
    let d = compile(src).expect_err("replace_literal must not be simulated by a host");
    assert_eq!(d.code, "E_UNSUPPORTED", "{d}");
    println!("ATTO_ROUTE_BLOCKER replace_literal={}", d.code);
}

#[test]
fn compiler_does_not_silently_link_external_module() {
    let src = r#"import module Router from "./http_routing.atto";
        pub fnc main(): int { return Router.answer(); }"#;
    let d = compile(src).expect_err("cross-file import must not resolve without linker");
    assert!(d.code.starts_with("E_"), "{d}");
    println!("ATTO_ROUTE_BLOCKER external_import={}:{}", d.code, d.message);
}
