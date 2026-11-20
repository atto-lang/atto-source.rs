//! Contract-only tests. No claim of compiled Atto FS guest execution.
use atto_compiler::{browser_fs_abi_v1 as abi, compile};
use abi::{FsError, Operation, PendingFs, Ticket};

#[test]
fn virtual_paths_reject_traversal_abs_roots_and_invalid_utf8() {
    for valid in ["./response.txt", "data/a.json", "é", "foo%2Fbar"] {
        assert!(abi::validate_relative_path(valid.as_bytes()).is_ok(), "{valid}");
    }
    for bad in ["", "./", "/etc/shadow", "a/../b", "a/./b",
        "a//b", "..", "a\\b", "x:y", "a\0b", "a\nb"] {
        assert_eq!(abi::validate_relative_path(bad.as_bytes()), Err(FsError::InvalidPath), "{bad:?}");
    }
    assert_eq!(abi::validate_relative_path(&[0xff]), Err(FsError::InvalidUtf8));
    assert_eq!(abi::validate_relative_path(&vec![b'x'; 257]), Err(FsError::InvalidPath));
}

#[test]
fn request_admission_checks_distinct_fs_permissions_and_generation() {
    let mut fs = PendingFs::new(11, 22, true, false, 2).unwrap();
    assert_eq!(fs.admit(1, Operation::Write, b"file", 1), Err(FsError::Permission));
    let first = fs.admit(1, Operation::Read, b"./a", 0).unwrap();
    assert_eq!(first.tenant, 11);
    assert_eq!(first.id.value(), 1);
    assert_eq!(fs.admit(1, Operation::Read, b"a", 0), Err(FsError::Duplicate));
    let second = fs.admit(2, Operation::Read, b"a", 0).unwrap();
    assert_eq!(fs.admit(3, Operation::Read, b"a", 0), Err(FsError::Busy));
    assert_eq!(fs.settle(Ticket {tenant: 12, ..first}), Err(FsError::Stale));
    assert_eq!(fs.settle(Ticket {incarnation: 21, ..first}), Err(FsError::Stale));
    assert_eq!(fs.pending(), 2);
    fs.settle(first).unwrap();
    assert_eq!(fs.settle(first), Err(FsError::Stale));
    assert_eq!(fs.revoke(), 1);
    assert_eq!(fs.settle(second), Err(FsError::Closed));
    assert_eq!(fs.admit(1, Operation::Read, b"a", 0), Err(FsError::Closed));
}

#[test]
fn no_host_memory_alias_or_bad_admission_can_pass() {
    assert_eq!(PendingFs::new(0, 1, true, true, 1).unwrap_err(), FsError::InvalidConfiguration);
    assert_eq!(PendingFs::new(1, 0, true, true, 1).unwrap_err(), FsError::InvalidConfiguration);
    assert_eq!(PendingFs::new(1, 1, true, true, 65).unwrap_err(), FsError::InvalidConfiguration);
    let mut fs = PendingFs::new(1, 1, true, true, 1).unwrap();
    assert_eq!(fs.admit(0, Operation::Read, b"a", 0), Err(FsError::InvalidId));
    assert_eq!(fs.admit(-1, Operation::Read, b"a", 0), Err(FsError::InvalidId));
    assert_eq!(fs.admit(1, Operation::Write, b"../x", 0), Err(FsError::InvalidPath));
    assert_eq!(fs.admit(1, Operation::Write, b"x", abi::MAX_REQUEST_BYTES + 1), Err(FsError::TooLarge));
    assert_eq!(fs.pending(), 0);
    assert!(abi::validate_guest_region(4, 8, 12, 12).is_ok());
    assert!(abi::validate_guest_region(12, 0, 12, 12).is_ok());
    assert_eq!(abi::validate_guest_region(11, 2, 12, 12), Err(FsError::OutOfBounds));
    assert_eq!(abi::validate_guest_region(u32::MAX, 2, u32::MAX, 12), Err(FsError::OutOfBounds));
    assert_eq!(abi::validate_guest_region(0, 3, 12, 2), Err(FsError::TooLarge));
}

#[test]
fn wire_schema_is_typed_and_normal_guest_builds_have_no_fs_import() {
    assert_eq!(abi::VERSION_SECTION, "atto.fs.v1");
    assert_eq!(abi::VERSION_MARKER, [1]);
    assert_eq!(abi::IMPORT_MODULE, "atto_fs_v1");
    assert_eq!(abi::IMPORT_READ, "read_request");
    assert_eq!(abi::IMPORT_WRITE, "write_request");
    assert_eq!(abi::IMPORT_CANCEL, "cancel");
    assert_eq!(abi::EXPORT_RESERVE, "atto_fs_v1_reserve");
    assert_eq!(abi::EXPORT_RESUME, "atto_fs_v1_resume");
    assert_eq!(abi::READ_PARAMS, [abi::ValueType::I32; 3]);
    assert_eq!(abi::WRITE_PARAMS, [abi::ValueType::I32; 5]);
    assert_eq!(abi::CANCEL_PARAMS, [abi::ValueType::I32]);
    assert_eq!(abi::RESERVE_PARAMS, [abi::ValueType::I32]);
    assert_eq!(abi::RESUME_PARAMS, [abi::ValueType::I32; 5]);

    let ordinary = compile("pub fnc main(): int { return 42; }").unwrap();
    assert!(ordinary.starts_with(b"\0asm"));
    assert!(!ordinary.windows(abi::IMPORT_MODULE.len())
        .any(|bytes| bytes == abi::IMPORT_MODULE.as_bytes()));
    let unimplemented = r#"import * as fs from pkg "std.fs";
pub susp fnc main(): int {
  await fs.write_text("./response.txt", "value");
  return 0;
}"#;
    assert!(compile(unimplemented).is_err(),
        "do not emit Wasm until FS package linker and real async guest lowering exist");
}
