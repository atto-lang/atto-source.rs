//! Candidate real browser compiler tests; executed guest fixtures are in pass/.
const GOOD: &str = include_str!("fixtures/pass/raw-pointer-slot-rebind.atto");

#[test]
fn reusable_slot_erases_to_ordinary_scalar_guest() {
    let actual = atto_compiler::compile(GOOD).unwrap();
    let plain = atto_compiler::compile("pub fnc main(): int { let a: int = 40; let b: int = 2; val first = a; val second = b; return first + second; }").unwrap();
    assert_eq!(actual, plain);
}

#[test]
fn reusable_slot_invalid_programs_fail_closed() {
    for (from, to) in [
        ("ref! p to a;", ""),
        ("unref! p;", "p = null;"),
        ("unref! p;", "unref! p; unref! p;"),
        ("unref! p;", ""),
        ("ref! p to a;", "ref! p to missing;"),
        ("ref! p to a;", "ref! p to a; ref! p to b;"),
        ("val first = deref! p;", "val first = deref! other;"),
        ("return first + second;", "return p;"),
        ("ref! p to a;", "ref ! p to a;"),
        ("let mut RawPointer<int>", "let mut RawPointer<string>"),
        ("unref! p;", "unref! p; await 42;"),
        ("unref! p;", "unref! p; val p = 0;"),
    ] {
        let bad = GOOD.replacen(from, to, 1);
        let err = atto_compiler::compile(&bad).expect_err("invalid slot program emitted Wasm");
        assert!(err.code.starts_with("E_POINTER_"), "{from} -> {to}: {err}");
    }
}

#[test]
fn erased_slot_cannot_hide_reserved_identifiers() {
    for name in ["await", "true", "null", "to", "empty"] {
        let source = format!("pub fnc main(): int {{ let mut RawPointer<int> {name} = RawPointer.empty<int>(); return 42; }}");
        let err = atto_compiler::compile(&source).expect_err("reserved slot name");
        assert!(err.code.starts_with("E_POINTER_"), "{name}: {err}");
    }
}

#[test]
fn erased_pointer_bang_operations_require_separators() {
    for (from, to) in [
        ("ref! p to a;", "ref!p to a;"),
        ("val first = deref! p;", "val first = deref!p;"),
        ("unref! p;", "unref!p;"),
    ] {
        assert!(GOOD.contains(from), "missing mutation target: {from}");
        let bad = GOOD.replacen(from, to, 1);
        let err = atto_compiler::compile(&bad).expect_err("glued operand emitted Wasm");
        assert!(err.code.starts_with("E_POINTER_") || err.code == "E_BANG_KEYWORD_WHITESPACE", "{from} -> {to}: {err}");
    }
}

#[test]
fn erased_linear_handle_cannot_hide_reserved_names() {
    for name in ["await", "true", "null", "to", "empty"] {
        let source = format!("pub fnc main(): int {{ let value: int = 42; val Pointer<int> {name} = ref! value; val answer = deref! {name}; unref! {name}; return answer; }}");
        assert!(atto_compiler::compile(&source).is_err(), "reserved handle emitted Wasm: {name}");
    }
}
