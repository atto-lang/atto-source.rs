//! Fail-closed compiler checks for ownership/synchronization intrinsics.
//! This is not executable guest Atomic, Arc or Box implementation coverage.
use atto_compiler::compile;

fn rejects_with_unsupported_type(source: &str) {
    let error = compile(source).expect_err("unlowered intrinsic must not emit Wasm");
    assert_eq!(error.code, "E_UNSUPPORTED_TYPE", "{error}");
}

#[test]
fn unlowered_atomic_arc_box_types_do_not_emit_guest_wasm() {
    for type_name in [
        "Atomic<int>", "AtomicRef<int>", "AtomicU64",
        "Arc<int>", "Box<int>", "Weak<int>", "Mutex<int>",
        "RwLock<int>", "Rc<int>", "UnsafeCell<int>",
    ] {
        rejects_with_unsupported_type(&format!(
            "pub fnc guarded(value: {type_name}): void {{ return; }}"
        ));
    }
}

#[test]
fn intrinsic_names_cannot_be_spoofed_as_guest_declarations() {
    for name in ["Atomic", "AtomicU64", "Arc", "Box", "Mutex", "Weak"] {
        rejects_with_unsupported_type(&format!("define class {name} as end"));
        rejects_with_unsupported_type(&format!("define actor {name} as end"));
        rejects_with_unsupported_type(&format!("define enum {name} as Example end"));
    }
}

#[test]
fn parse_only_target_signatures_do_not_launder_intrinsic_names() {
    rejects_with_unsupported_type("use namespace workers; declare actor Arc;");
    rejects_with_unsupported_type(
        "use namespace workers; declare actor Worker; \
         mtd (Worker self) handle(Atomic<int> value): void { return; }"
    );
}

#[test]
fn ordinary_scalar_guest_remains_executable() {
    let wasm = compile("pub fnc main(): int { return 17; }")
        .expect("safe scalar subset must remain enabled");
    assert_eq!(&wasm[..8], b"\0asm\x01\0\0\0");
}

#[test]
fn ordinary_user_named_cell_is_not_an_atomic_intrinsic() {
    // The unlowered-atomic guard must not collide with an ordinary Atto class.
    let source = "define class Cell as pub int value = 1; end pub fnc main(): int { return 42; }";
    let guest = compile(source).expect("user-defined Cell must stay valid");
    assert_eq!(&guest[0..4], &[0, 97, 115, 109]); // Wasm binary magic
}
