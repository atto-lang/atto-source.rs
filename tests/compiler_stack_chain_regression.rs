//! Keep long bounded chains and branches executable; reject over-budget
//! source without aborting the compiler. No test stack-size increase.
use atto_compiler::compile;
use std::{io::Write, process::{Command, Stdio}};

// Execute compiler-produced bytes, not just a Wasm magic-header check.
fn run_guest(bytes: &[u8], expected: &str) {
    let mut child = Command::new("node")
        .args(["-e", r#"const assert=require('node:assert/strict');
const m=new WebAssembly.Module(require('node:fs').readFileSync(0));
const imports={};for(const i of WebAssembly.Module.imports(m)){
  assert.equal(i.kind,'function');(imports[i.module]??={})[i.name]=()=>{throw Error('unexpected host call '+i.name)};
}
const g=new WebAssembly.Instance(m,imports).exports;
assert.equal(typeof g.main,'function');
if(process.argv[1]==='trap'){assert.throws(()=>g.main(),WebAssembly.RuntimeError);}else{assert.equal(g.main(),BigInt(process.argv[1]));}"#, expected])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().expect("Node Wasm engine is required");
    child.stdin.take().expect("guest stdin").write_all(bytes).expect("write emitted Wasm");
    let result=child.wait_with_output().expect("wait for real guest");
    assert!(result.status.success(),"guest execution: {:?}",result);
    assert!(result.stdout.is_empty() && result.stderr.is_empty(),"unexpected guest output: {:?}",result);
}

#[test]
fn long_chain_on_small_host_stack() {
    let handle=std::thread::Builder::new().stack_size(2*1024*1024).spawn(|| {
        let chain=|n:usize|format!("pub fnc main(): int {{ return {}; }}",vec!["1";n].join(" + "));
        let guest=compile(&chain(250)).expect("250 terms must remain admitted");
        assert_eq!(&guest[..8],b"\0asm\x01\0\0\0");
        run_guest(&guest, "250");
        let overflow=format!("pub fnc main(): int {{ return 9223372036854775807 + 1 - {}; }}", vec!["1";248].join(" - "));
        let guest=compile(&overflow).expect("bounded checked arithmetic must emit guest");
        run_guest(&guest, "trap");
        assert_eq!(compile(&chain(3000)).unwrap_err().code,"E_NESTING");
    }).unwrap();
    handle.join().unwrap();
}
#[test]
fn long_elif_on_small_host_stack() {
    let handle=std::thread::Builder::new().stack_size(2*1024*1024).spawn(|| {
        let make=|n:usize| {
            let arms=(1..n).map(|i|format!("elif x eq {i} then return {i};")).collect::<Vec<_>>().join("\n");
            format!("pub fnc main(): int {{\nval x = 249;\nif x eq 0 then return 0;\n{arms}\nfi\nreturn 0;\n}}")
        };
        let guest=compile(&make(250)).expect("250 branches must remain admitted");
        assert_eq!(&guest[..8],b"\0asm\x01\0\0\0");
        run_guest(&guest, "249");
        assert_eq!(compile(&make(300)).unwrap_err().code,"E_NESTING");
    }).unwrap();
    handle.join().unwrap();
}
