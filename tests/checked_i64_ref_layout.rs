//! Real native executions for checked-i64 compilation across internal
//! managed-reference layouts. Scalar source has no pointer instructions yet.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use oreslang_source_rs::{
    compile_checked_i64, compile_checked_i64_with_ref_layout,
    ref_layout::ManagedRefLayout,
};
use std::{
    fs,
    os::unix::process::ExitStatusExt,
    path::PathBuf,
    process::{Command, ExitStatus},
    time::{SystemTime, UNIX_EPOCH},
};

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "oreslang-checked-layout-{}-{now}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create native guest directory");
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run_guest(source: &str, layout: ManagedRefLayout) -> ExitStatus {
    let dir = Scratch::new();
    let object = dir.0.join("guest.o");
    let executable = dir.0.join("guest");
    let bytes = compile_checked_i64_with_ref_layout(source, layout)
        .unwrap_or_else(|err| panic!("checked compilation failed for {layout:?}: {err}"));
    assert!(bytes.starts_with(b"\x7fELF"), "expected Linux native object");
    fs::write(&object, bytes).expect("write object");

    let linked = Command::new("cc")
        .arg(&object)
        .arg("-o")
        .arg(&executable)
        .output()
        .expect("invoke native linker");
    assert!(
        linked.status.success(),
        "linker failed for {layout:?}: {}",
        String::from_utf8_lossy(&linked.stderr)
    );
    Command::new(&executable)
        .status()
        .expect("execute native checked guest")
}

#[test]
fn checked_i64_native_and_compressed_layouts_keep_real_traps() {
    let ordinary = include_str!("fixtures/checked_i64_v1/signed.atto");
    let overflow = include_str!("fixtures/checked_i64_v1/dynamic-add.atto");
    for layout in [
        ManagedRefLayout::native(),
        ManagedRefLayout::default(),
        ManagedRefLayout::compressed32(0).unwrap(),
        ManagedRefLayout::compressed32(15).unwrap(),
    ] {
        let ok = run_guest(ordinary, layout);
        assert_eq!(ok.code(), Some(42), "{layout:?}: {ok:?}");

        let trap = run_guest(overflow, layout);
        assert_eq!(
            trap.signal(),
            Some(4),
            "overflow must trap with SIGILL, never wrap: {layout:?}: {trap:?}"
        );
    }
}

#[test]
fn explicit_layout_does_not_bypass_frontend_or_mir_admission() {
    for layout in [ManagedRefLayout::native(), ManagedRefLayout::default()] {
        for source in [
            "pub fnc main(): int { return ghost; }",
            "define actor Worker as end",
        ] {
            assert!(
                compile_checked_i64_with_ref_layout(source, layout).is_err(),
                "invalid source passed for {layout:?}: {source}"
            );
        }
    }
}

#[test]
fn old_default_entrypoint_remains_available() {
    let source = "pub fnc main(): int { return 42; }";
    let default_object = compile_checked_i64(source).expect("default checked build");
    let configured_object = compile_checked_i64_with_ref_layout(
        source,
        ManagedRefLayout::default(),
    )
    .expect("explicit default layout checked build");
    assert!(!default_object.is_empty() && !configured_object.is_empty());
}
