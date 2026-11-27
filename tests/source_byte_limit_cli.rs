//! Browser CLI byte boundary; the historical native AOT CLI has a separate
//! target and contract, preserved under legacy-native rather than fabricated.
use atto_compiler::SOURCE_CAP;
use std::{fs,path::PathBuf,process::Command,time::{SystemTime,UNIX_EPOCH}};
struct Scratch(PathBuf);
impl Drop for Scratch { fn drop(&mut self) { let _=fs::remove_dir_all(&self.0); } }
fn scratch() -> Scratch {
    let dir=std::env::temp_dir().join(format!("atto-browser-bytes-{}-{}",std::process::id(),SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
    fs::create_dir(&dir).expect("exclusive scratch directory");
    Scratch(dir)
}
fn source(n:usize)->String {
    let prefix="pub fnc main(): int { return 42; }\n//é";
    format!("{prefix}{}", "x".repeat(n-prefix.len()))
}
fn run(args:&[&str])->std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_attoc")).args(args).output().expect("actual browser compiler CLI")
}
#[test]
fn exact_utf8_byte_limit_accepts_check_and_emits_wasm() {
    let d=scratch(); let input=d.0.join("input.atto"); let out=d.0.join("out.wasm");
    let text=source(SOURCE_CAP); assert_eq!(text.len(),SOURCE_CAP); fs::write(&input,text).unwrap();
    let check=run(&["--check",input.to_str().unwrap()]);
    assert!(check.status.success(),"{:?}",check);
    let compiled=run(&[input.to_str().unwrap(),"-o",out.to_str().unwrap()]);
    assert!(compiled.status.success(),"{:?}",compiled);
    assert!(fs::read(out).unwrap().starts_with(b"\0asm"));
}
#[test]
fn oversized_utf8_source_never_publishes_guest_bytes() {
    let d=scratch(); let input=d.0.join("input.atto"); let out=d.0.join("out.wasm");
    let text=source(SOURCE_CAP+1); assert_eq!(text.len(),SOURCE_CAP+1); fs::write(&input,text).unwrap();
    for args in [vec!["--check",input.to_str().unwrap()],vec![input.to_str().unwrap(),"-o",out.to_str().unwrap()],vec![input.to_str().unwrap()]] {
        let result=run(&args);
        assert!(!result.status.success(),"{:?}",args);
        assert!(String::from_utf8_lossy(&result.stderr).contains("E_SOURCE_LIMIT"),"{:?}",result);
        assert!(result.stdout.is_empty()); assert!(!out.exists());
    }
}
