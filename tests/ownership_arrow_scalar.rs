//! Initial scalar admission only; does not qualify non-Copy lifetime behavior.
use atto_compiler::compile;

fn program(body: &str) -> String {
    format!("pub fnc main(): int {{ {body} }}")
}

#[test]
fn scalar_sources_remain_live() {
    for body in [
        "let b = 21; let a <- b; return a + b;",
        "let b = 21; let int a <- b; return a + b;",
        "let a = 0; let b = 21; set a <- b; return a + b;",
        "let a = 0; set a <- 42; return a;",
        "let a = 0; set a <- 21 + 21; return a;",
        "let a = 0; let b = 21; set a = b; return a + b;",
    ] {
        let wasm = compile(&program(body)).expect(body);
        assert_eq!(&wasm[..8], b"\0asm\x01\0\0\0");
    }
}

#[test]
fn invalid_arrow_binding_rejected() {
    for (body, code) in [
        ("let b = 42; set a <- b; return 0;", "E_UNBOUND"),
        ("const a = 0; let b = 42; set a <- b; return a;", "E_IMMUTABLE"),
        ("let a = 0; let b = true; set a <- b; return a;", "E_TYPE"),
        ("let a = 42; set a <- a; return a;", "E_OWNERSHIP"),
    ] {
        let error = compile(&program(body)).expect_err(body);
        assert_eq!(error.code, code, "{body}: {error}");
    }
    for body in [
        "let b = 42; const a <- b; return a;",
        "let b = 42; int a <- b; return a;",
        "let b = 42; val a <- b; return a;",
        "let a = 0; let b = 42; a <- b; return a;",
        "let b = 42; let a • b; return a;",
        "let b = 42; let a < - b; return a;",
        "let a = 0; let b = 42; set! a <- b; return a;",
    ] {
        assert!(compile(&program(body)).is_err(), "{body}");
    }
}

#[test]
fn non_copy_transfer_fails_closed() {
    let source = "define class Parcel as pub let int value = 42; end pub fnc main(): int { let b = new Parcel(); let a <- b; return a.value; }";
    let error = compile(source).expect_err("non-Copy transfer needs cleanup proof");
    assert_eq!(error.code, "E_OWNERSHIP", "{error}");
}
