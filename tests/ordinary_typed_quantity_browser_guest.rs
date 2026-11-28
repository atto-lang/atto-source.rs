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
    // #846 admits standalone alias-free quantity sources via checked IR.
    let alias_free = "define dimension Time; define unit Second: Time; pub fnc main() -> int { return 42; }";
    let guest = compile(alias_free).expect("alias-free checked IR");
    assert!(guest.starts_with(b"\0asm"));
    assert_eq!(atto_compiler::compile_verified_strict(alias_free).unwrap_err().code, "E_VERIFY_UNSUPPORTED");
    assert_eq!(compile("define dimension Time; define unit Second: Time; pub fnc main() -> int { return 42<Second>; }").unwrap_err().code, "E_UNIT_ERASURE");
    assert!(compile("define dimension Time; define unit Second: Time; pub fnc main() -> int { return 42; } trailing").is_err());
    assert!(compile("pub fnc main() -> int { return 42; }").is_ok());
    let malformed=format!("{PRE}\npub fnc main() -> int {{ return 42; }} trailing");
    assert!(compile(&malformed).is_err());
}

#[test]
fn import_capable_entry_preserves_file_local_quantity_guest_without_loading() {
    use atto_compiler::compile_with_imports;
    use std::path::Path;
    let source = format!("{PRE}\npub fnc main() -> int {{ val Positive a = 42<Second>; return a / 1<Second>; }}");
    let expected = compile(&source).expect("ordinary typed source");
    let actual = compile_with_imports(&source, Path::new("main.atto"), |_| panic!("file-local quantity must not load dependencies")).expect("CLI-linked typed source");
    assert_eq!(actual, expected);
    let malformed = format!("{source} import other from './other.atto';");
    assert!(compile_with_imports(&malformed, Path::new("main.atto"), |_| panic!("unsupported quantity import must not load dependencies")).is_err());
}
#[test]
fn actual_cli_emits_same_typed_quantity_guest() {
    let source = include_str!("fixtures/typed-quantity-cli.atto");
    let expected = compile(source).expect("ordinary typed source");
    let output = Command::new(env!("CARGO_BIN_EXE_attoc"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .arg("tests/fixtures/typed-quantity-cli.atto")
        .output().expect("attoc must be launchable");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(output.stdout, expected);
    assert!(output.stderr.is_empty());
}
