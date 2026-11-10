use std::{fs,process::Command};
#[test]
fn browser_targets_use_canonical_core_and_reject_without_output() {
    let dir=std::env::temp_dir().join(format!("ores-browser-cli-{}",std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let input=dir.join("main.atto");
    let source="define actor Counter as let int n = 0; receive(ActorMail<int> mail): void { self.n = self.n + mail.value; return; } end pub fnc main(): int { val c = spawn Counter(); c.send(42); return 0; }";
    fs::write(&input,source).unwrap();
    for (flag,name,want) in [("--wasm","main.wasm",oreslang_source_rs::compile_browser(source).unwrap()),("--js","main.mjs",oreslang_source_rs::transpile_browser(source).unwrap().into_bytes())] {
        let out=dir.join(name);
        let r=Command::new(env!("CARGO_BIN_EXE_oresc")).args([flag,input.to_str().unwrap(),out.to_str().unwrap()]).output().unwrap();
        assert!(r.status.success(),"{}",String::from_utf8_lossy(&r.stderr));
        assert_eq!(fs::read(&out).unwrap(),want);
        fs::remove_file(&out).unwrap();
        let r=Command::new(env!("CARGO_BIN_EXE_oresc")).args(["--strict-aot",flag,input.to_str().unwrap(),out.to_str().unwrap()]).output().unwrap();
        assert!(!r.status.success());assert!(!out.exists());
    }
    fs::write(&input,"pub fnc main(): int { return missing; }").unwrap();
    let out=dir.join("rejected.wasm");
    assert!(!Command::new(env!("CARGO_BIN_EXE_oresc")).args(["--wasm",input.to_str().unwrap(),out.to_str().unwrap()]).status().unwrap().success());
    assert!(!out.exists());
    fs::remove_dir_all(dir).unwrap();
}
