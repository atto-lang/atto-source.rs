//! Ordinary browser entry must retain typed quantity/refinement guest lowering.
use atto_compiler::compile;
use std::{io::Write, process::{Command, Stdio}};
const PRE: &str = "define dimension Time; define unit Second: Time; define unit Millis: Time = Second / 1000; type Positive = int where self > 0 and units in Second; type Bounded = int where self >= 1 and self <= 42;";
fn run(expr: &str, expected: &str, panic_code: &str) {
    let source = format!("{PRE}\npub fnc main() -> int {{ return {expr}; }}");
    let bytes = compile(&source).unwrap_or_else(|d| panic!("{expr}: {d}"));
    let mut child = Command::new("node").args(["-e", r#"
const assert=require('node:assert/strict');
const m=new WebAssembly.Module(require('node:fs').readFileSync(0));
assert.deepEqual(WebAssembly.Module.imports(m),[]);
const g=new WebAssembly.Instance(m).exports;
assert.equal(g.panic_code.value,0);
if(process.argv[1]==='trap')assert.throws(()=>g.main(),WebAssembly.RuntimeError);
else assert.equal(g.main(),BigInt(process.argv[1]));
assert.equal(g.panic_code.value,Number(process.argv[2]));
"#, expected, panic_code]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().expect("real Wasm engine required");
    child.stdin.take().unwrap().write_all(&bytes).unwrap();
    let result=child.wait_with_output().unwrap();
    assert!(result.status.success(),"{expr}: {result:?}");
}
#[test]
fn typed_quantity_ordinary_guests_execute() {
    for expr in ["<Positive>(42.<Second>) / 1<Second>", "(42.<Second>)::<Positive> / 1<Second>", "(42.<Second> as Positive) / 1<Second>"] {
        run(expr,"42","0");
    }
    run("<Positive>(0.<Second>) / 1<Second>","trap","7");
    run("<Bounded>(1<Second> / 0<Second>)","trap","2");
    run("(9223372036854775807 + 1) as Bounded","trap","1");
}
#[test]
fn typed_quantity_errors_fail_closed_without_changing_unrelated_admission() {
    for (expr, code) in [("<Positive>(42<Millis>)","E_UNIT_SCALE"),("<i64>(42<Second>)","E_UNIT_ERASURE")] {
        let source=format!("{PRE}\npub fnc main() -> int {{ return {expr}; }}");
        assert_eq!(compile(&source).unwrap_err().code,code, "{expr}");
    }
    assert!(compile("define dimension Time; define unit Second: Time; pub fnc main() -> int { return 42; }").is_err());
    assert!(compile("pub fnc main() -> int { return 42; }").is_ok());
    let malformed=format!("{PRE}\npub fnc main() -> int {{ return 42; }} trailing");
    assert!(compile(&malformed).is_err());
}
