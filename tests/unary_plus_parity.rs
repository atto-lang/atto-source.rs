//! Signed unary plus: browser compiler edition. The migrated predecessor
//! targeted a different native ELF compiler crate (oreslang_source_rs).
//! Retain the semantic checks against this repository's actual Wasm ABI;
//! native ELF qualification remains the responsibility of native backends.
use atto_compiler::compile;

fn execute(source: &str, expected: &str) {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let bytes = compile(source).unwrap_or_else(|d| panic!("{source}: {d}"));
    assert_eq!(&bytes[..8], b"\0asm\x01\0\0\0");
    let script = r#"
        const fs=require('node:fs');
        const bytes=fs.readFileSync(0);
        const mod=new WebAssembly.Module(bytes);
        const imports={atto:{}};
        for(const entry of WebAssembly.Module.imports(mod)){
          if(entry.module!=='atto'||entry.kind!=='function')throw Error('unexpected import');
          imports.atto[entry.name]=()=>{throw Error('unexpected host call '+entry.name);};
        }
        const guest=new WebAssembly.Instance(mod,imports);
        if(guest.exports.main()!==BigInt(process.argv[1]))throw Error('wrong result');
        if(guest.exports.panic_code.value!==0)throw Error('guest panic');
    "#;
    let mut child=Command::new("node").args(["-e",script,expected])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().expect("Node is required for guest execution");
    child.stdin.take().unwrap().write_all(&bytes).unwrap();
    let output=child.wait_with_output().unwrap();
    assert!(output.status.success(), "{}",String::from_utf8_lossy(&output.stderr));
}

#[test]
fn unary_plus_preserves_integer_literals_and_nested_precedence() {
    execute("pub fnc main(): int { return +(+40) + +2; }","42");
    execute("pub fnc main(): int { return +42; }","42");
}

#[test]
fn unary_plus_handles_calls_and_checked_codegen() {
    execute("pub fnc main(): int { return +add(+19, +23); } fnc add(int left, int right): int { return left + right; }","42");
}

#[test]
fn unary_plus_never_bypasses_i64_bounds_or_name_checking() {
    let d=compile("pub fnc main(): int { return +9223372036854775808; }").unwrap_err();
    assert_eq!(d.code,"E_INTEGER_RANGE");
    let d=compile("pub fnc main(): int { return ++9223372036854775808; }").unwrap_err();
    assert_eq!(d.code,"E_UNSUPPORTED");
    let d=compile("pub fnc main(): int { return +not_declared; }").unwrap_err();
    assert_eq!(d.code,"E_UNBOUND");
}

#[test]
fn unary_plus_executes_full_i64_minimum_in_wasm_guest() {
    execute("pub fnc main(): int { return +(-9223372036854775808); }","-9223372036854775808");
}
