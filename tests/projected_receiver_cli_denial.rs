//! CLI-level fail-closed proof: rejected actor receiver programs must never
//! materialize a Wasm artifact, even with an explicit -o output path.
//! This tests the real attoc executable, NOT actor guest execution.
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let ticks = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!(
            "atto-receiver-denial-{}-{ticks}-{id}", std::process::id()
        ));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run_attoc(src: &str, workdir: &Path, output: &Path) -> Output {
    let input = workdir.join("input.atto");
    std::fs::write(&input, src).unwrap();
    Command::new(env!("CARGO_BIN_EXE_attoc"))
        .arg(&input)
        .arg("-o")
        .arg(output)
        .current_dir(workdir)
        .output()
        .expect("attoc must run")
}

#[test]
fn rejected_named_actor_state_access_never_writes_wasm_artifact() {
    let invalid = [
        ("nested_state_read", "E_SEALED",
            "declare actor Foo; mtd (Foo this) f(): void { this.send(this.state.counter); }"),
        ("receiver_escape", "E_ESCAPE",
            "declare actor Foo; mtd (Foo this) f(): void { this.send(this); }"),
        ("unguarded_write", "E_SEALED",
            "declare actor Foo; mtd (Foo mut this.state.counter) f(): void { this.state.counter = 1; }"),
        ("unauthorized_write", "E_MUTABILITY",
            "declare actor Foo; mtd (Foo mut this.state.counter) f(): void { this.handle_state(critical-write |s| -> { s.other = 1; }); }"),
        ("nested_state_projection", "E_UNSUPPORTED",
            "declare actor Foo; mtd (Foo mut this.state.counter) f(): void { this.handle_state(critical-write |s| -> { s.other.inner = 1; }); }"),
        ("declared_but_not_guest_lowered", "E_UNSUPPORTED",
            "declare actor Foo; mtd (Foo mut this.state.counter) f(): void { this.handle_state(critical-write |s| -> { s.counter += 1; }); }"),
        ("exact_playground_preview", "E_UNSUPPORTED",
            include_str!("playground_previews/actor-named-state-permissions.atto")),
    ];
    for (name, code, src) in invalid {
        let temp = Scratch::new();
        let output = temp.0.join(format!("{name}.wasm"));
        let result = run_attoc(src, &temp.0, &output);
        assert_eq!(result.status.code(), Some(1),
            "{name} unexpectedly compiled: {result:?}");
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert!(stderr.contains(code), "{name} wrong diagnostic: {stderr}");
        assert!(!output.exists(), "{name} emitted an artifact on diagnostic");
    }
}

#[test]
fn same_cli_still_emits_a_real_wasm_module_for_supported_scalar_code() {
    let temp = Scratch::new();
    let output = temp.0.join("scalar.wasm");
    let result = run_attoc("pub fnc main(): int { return 42; }", &temp.0, &output);
    assert!(result.status.success(), "scalar compile failed: {result:?}");
    let wasm = std::fs::read(&output).expect("successful compile writes Wasm");
    assert!(wasm.len() >= 8, "short compiled output");
    assert_eq!(&wasm[..4], b"\0asm", "not a WebAssembly module");
}
