//! Explicit native u64 profile. A native trap is not a typed guest Error.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use oreslang_source_rs::{compile_checked_i64, compile_checked_u64};
use std::{fs, os::unix::process::ExitStatusExt, path::PathBuf, process::Command, sync::atomic::{AtomicUsize, Ordering}};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Temp(PathBuf);
impl Drop for Temp {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}

fn execute(source: &str) -> std::process::Output {
    let dir = Temp(std::env::temp_dir().join(format!("atto-u64-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed))));
    fs::create_dir(&dir.0).unwrap();
    let bytes = compile_checked_u64(source).unwrap();
    assert!(bytes.starts_with(b"\x7fELF"));
    fs::write(dir.0.join("guest.o"), bytes).unwrap();
    fs::write(dir.0.join("observer.c"), "#include <stdint.h>\n#include <inttypes.h>\n#include <stdio.h>\nextern uint64_t atto_guest_main(void);\nint main(void){printf(\"%\" PRIu64 \"\\n\", atto_guest_main());return 0;}\n").unwrap();
    let link = Command::new("cc").current_dir(&dir.0).args(["observer.c", "guest.o", "-o", "guest"]).output().unwrap();
    assert!(link.status.success(), "{:?}", link);
    Command::new(dir.0.join("guest")).current_dir(&dir.0).output().unwrap()
}

#[test]
fn exact_full_width_results_and_faults() {
    for line in include_str!("../../contracts/atto-xo-checked-u64-debit-v1/cases.tsv").lines() {
        let fields: Vec<_> = line.split('|').collect();
        assert_eq!(fields.len(), 9);
        let output = execute(fields[8]);
        if fields[1] == "overflow" {
            assert_eq!(output.status.signal(), Some(4), "{:?}", fields[0]);
            assert!(output.stdout.is_empty());
        } else {
            assert!(output.status.success(), "{:?}", fields[0]);
            assert_eq!(String::from_utf8(output.stdout).unwrap(), format!("{}\n", fields[1]));
            assert!(output.stderr.is_empty());
        }
    }
}

#[test]
fn signed_and_unsigned_admission_remain_separate() {
    for source in [
        "pub fnc main(): u64 { return 18446744073709551616; }",
        "pub fnc main(): u64 { return -1; }",
        "pub fnc main(): int { return 1; }",
        "pub fnc main(): u64 { return f(1); } fnc f(int x): u64 { return x; }",
        "pub fnc main(): u64 { return 1; } fnc atto_guest_main():u64 { return 1; }",
        "fnc main(): u64 { return 1; }",
        "pub fnc main(u64 x): u64 { return x; }",
    ] { assert!(compile_checked_u64(source).is_err(), "{source}"); }
    assert!(compile_checked_i64("pub fnc main():u64{return 1;}").is_err());
}

#[test]
fn unsigned_call_graph_is_closed_and_bounded() {
    for source in [
        "pub fnc main():u64{return main();}",
        "pub fnc main():u64{return 42;} fnc a():u64{return b();} fnc b():u64{return a();}",
        "pub fnc main():u64{return 42;} fnc unused():u64{return unused();}",
    ] {
        assert!(compile_checked_u64(source).unwrap_err().contains("recursive unsigned call graph"));
    }
    let mut source = String::from("fnc f0():u64{return 0;}");
    for i in 1..=14 {
        source.push_str(&format!("fnc f{i}():u64{{return f{}()+f{}();}}",i-1,i-1));
    }
    source.push_str("pub fnc main():u64{return f14();}");
    assert!(compile_checked_u64(&source).unwrap_err().contains("expanded work budget"));
    let result=execute("pub fnc main():u64{return f(20)+f(21);} fnc f(u64 n):u64{return n+1;}");
    assert!(result.status.success());
    assert_eq!(result.stdout,b"43\n");
}
