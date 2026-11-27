//! Production guest tests, not parser-only assertions.
use atto_compiler::compile_namespace_file_v2;
use std::io::Write;
use std::process::{Command, Stdio};

const HEADER: &str = "declare pub[pkg] class Foo;";
const CTOR: &str = "pub constructor (Foo) () {}";
const METHOD: &str = "pub mtd (Foo) answer(): int { return 42; }";
const MAIN: &str = "pub fnc main(): int { val foo = new Foo(); return foo.answer(); }";

fn execute(source: &str) {
    let wasm = compile_namespace_file_v2(source).expect("checked declared class must compile");
    assert_eq!(&wasm[..8], b"\0asm\x01\0\0\0");
    let mut child = Command::new("node").arg("-e").arg(
        "const fs=require('node:fs');WebAssembly.instantiate(fs.readFileSync(0),{}).then(({instance})=>{if(instance.exports.main()!==42n)throw Error('guest mismatch');}).catch(e=>{console.error(e);process.exitCode=1;});"
    ).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().expect("Node is required for actual namespace guest execution");
    child.stdin.take().unwrap().write_all(&wasm).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}

#[test]
fn file_spellings_and_source_order_execute_same_declared_class() {
    for prefix in ["", "use file namespace;", "use default namespace;"] {
        for group in [
            format!("{HEADER}\n{CTOR}\n{METHOD}\n{MAIN}"),
            format!("{MAIN}\n{METHOD}\n{CTOR}\n{HEADER}"),
            format!("{HEADER}\npub constructor (Foo) (): self {{}}\n{METHOD}\n{MAIN}"),
        ] {
            execute(&format!("{prefix}\n{group}"));
        }
    }
}

#[test]
fn class_completion_rejects_incomplete_duplicate_foreign_and_mixed_owners() {
    for (source, code) in [
        (format!("{HEADER}\npub fnc main(): int {{return 42;}}"), "E_UNLINKED"),
        (format!("{HEADER}\n{CTOR}\n{METHOD}\n{METHOD}\n{MAIN}"), "E_DUPLICATE"),
        (format!("{HEADER}\n{CTOR}\n{CTOR}\n{METHOD}\n{MAIN}"), "E_DUPLICATE"),
        (format!("{HEADER}\nconstructor (Other) () {{}}\n{METHOD}\n{MAIN}"), "E_UNBOUND"),
        (format!("{HEADER}\ndefine class Foo as end\n{METHOD}\n{MAIN}"), "E_UNSUPPORTED"),
        (format!("{HEADER}\n{CTOR}\n{METHOD}\ndeclare class Missing;\n{MAIN}"), "E_UNLINKED"),
    ] {
        let error = compile_namespace_file_v2(&source).expect_err("must reject before emission");
        assert_eq!(error.code, code, "{source}: {error}");
    }
}

#[test]
fn ordinary_ownership_and_typing_still_gate_synthetic_layout() {
    for constructor in [
        "constructor (Foo) () { val alias = self; }",
        "constructor (Foo) () { self.answer(); }",
        "constructor (Foo) () { self.missing = 42; }",
    ] {
        let source = format!("{HEADER}\n{constructor}\n{METHOD}\n{MAIN}");
        assert!(compile_namespace_file_v2(&source).is_err(), "{source}");
    }
    let wrong = format!("{HEADER}\n{CTOR}\npub mtd (Foo) answer(): int {{ return true; }}\n{MAIN}");
    assert!(compile_namespace_file_v2(&wrong).is_err());
    let mutable = format!("{HEADER}\n{CTOR}\npub mtd (mut self: Foo) answer(): int {{ return 42; }}\n{MAIN}");
    assert_eq!(compile_namespace_file_v2(&mutable).unwrap_err().code, "E_UNSUPPORTED");
}
