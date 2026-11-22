//! Return-shorthand conformance on the real Rust -> Wasm compiler.
//! These tests require actual Cargo execution; source existence is not a pass receipt.
use atto_compiler::compile;

fn accepts(source: &str) {
    let wasm = compile(source).unwrap_or_else(|error|
        panic!("expected valid Atto guest, got {error}\n{source}"));
    assert_eq!(&wasm[..8], b"\0asm\x01\0\0\0", "must emit Wasm");
}

const ENTRY: &str = "pub fnc main(): int { early_exit(); ^ 42; }";

#[test]
fn void_early_returns_accept_both_if_families() {
    for body in [
        "if true { ^ }",
        "if true { ^; }",
        "if true { ^ void }",
        "if true { ^ void; }",
        "if true then ^ fi",
        "if true then ^ void fi",
        "do if true then ^ fi",
        "do if true then ^ void fi",
        "do if false then ^ void fi",
        "if false { ^ void; }",
        "return void;",
        "^ void;",
        "^;",
        "if true { ^ void; } fi", // explicit legacy fi preserved
        "if true then if true { ^ void; } fi",
        "if true then if true { ^ void; } fi fi", // two explicit fi close both
        "if true { if true then ^ void fi }",
    ] {
        let source = format!("fnc early_exit() -> void {{ {body} }} {ENTRY}");
        accepts(&source);
    }
}

#[test]
fn caret_void_nonvoid_return_is_rejected_by_type_checker() {
    for source in [
        "pub fnc main(): int { ^; }",
        "pub fnc main(): int { ^ void; }",
        "pub fnc main(): int { return void; }",
        "pub fnc main(): int { if true { ^ void; } ^ 42; }",
        "pub fnc main(): int { do if true then ^ void fi ^ 42; }",
    ] {
        let err = compile(source).expect_err("nonvoid return must never emit guest Wasm");
        assert_eq!(err.code, "E_RETURN", "{source}: {err}");
    }
}

#[test]
fn explicit_void_marker_is_not_an_expression() {
    for source in [
        "pub fnc main(): int { val x = void; ^ 42; }",
        "fnc early_exit() -> void { ^ void + 1; } pub fnc main(): int { early_exit(); ^ 42; }",
        "fnc early_exit() -> void { ^ void 42; } pub fnc main(): int { early_exit(); ^ 42; }",
        "pub fnc main(): int { ^ 12 ^ 10; }", // obsolete single-caret XOR
    ] {
        compile(source).expect_err("invalid marker/infix operation must reject");
    }
    // Canonical XOR is contiguous ^^ and is still an ordinary int value.
    accepts("pub fnc main(): int { ^ 12 ^^ 10; }");
}

#[test]
fn braced_if_must_not_steal_enclosing_fi() {
    // For the nested braced if, a single fi belongs to the outer keyword if.
    accepts(&format!(
        "fnc early_exit() -> void {{ if true then if true {{ ^ void; }} fi }} {ENTRY}"
    ));
    // A braced standalone if may still use a trailing fi for compatibility.
    accepts(&format!(
        "fnc early_exit() -> void {{ if true {{ ^ void; }} fi }} {ENTRY}"
    ));
}
