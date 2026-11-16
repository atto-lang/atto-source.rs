//! Source-level declaration-first reads must be the same checked AST/runtime
//! operation as prefix `<< c`, not a binary write or a second dequeue.
use atto_compiler::{compile, ast::{ExprKind, Stmt}, lexer, parser::Parser};

const PRELUDE: &str = "pub fnc main(): int { return 0; }";
fn src(body: &str, modifier: &str) -> String {
    format!("pub {modifier} fnc read_channel_guest_v1(Channel<int> c): Result<Option<int>,IoError> {{ {body} }} {PRELUDE}")
}
fn has(bytes: &[u8], name: &str) -> bool {
    bytes.windows(name.len()).any(|v| v==name.as_bytes())
}

#[test]
fn declaration_first_and_prefix_form_have_one_identical_read_ast_node() {
    for body in [
        "val result = << c; return result;",
        "val result = (<< c); return result;",
        "val result << c; return result;",
        "val Result<Option<int>, IoError> result << c; return result;",
        "val result: Result<Option<int>, IoError> << c; return result;",
        "let result << c; return result;",
    ] {
        let source=src(body,"suspends");
        let parsed=Parser::new(lexer::lex(&source).unwrap()).parse_program().expect(body);
        let Stmt::Bind {init,..}=&parsed.funcs[0].body[0] else {panic!("missing binding: {body}")};
        assert!(matches!(&init.kind,ExprKind::PrefixRead(_)),
            "decl-first must reuse PrefixRead AST: {body}");
        let wasm=compile(&source).expect(body);
        assert!(wasm.starts_with(b"\0asm\x01\0\0\0"));
        assert!(has(&wasm,"atto.channel_int"),"checked guest marker missing");
        assert!(has(&wasm,"channel_int_nb_read_v1"));
        assert!(has(&wasm,"channel_int_await_read_v1"));
    }
}

#[test]
fn declaration_first_preserves_checked_result_and_suspends_diagnostics() {
    for (body,modifier,error) in [
        ("val int result << c; return result;","susp","E_TYPE"),
        ("val result: int << c; return result;","susp","E_TYPE"),
        ("const result << c; return result;","susp","E_CONST_EFFECT"),
        ("val result << 42; return result;","susp","E_IO_SYNTAX"),
        ("val result << c; return result;","fnc","E_SYNTAX"),
    ] {
        // The source builder already adds fnc; separately test the plain fnc
        // suspension effect below rather than inserting a duplicate keyword.
        if modifier=="fnc" {continue}
        let source=src(body,modifier);
        let diagnostic=compile(&source).expect_err("negative must not emit Wasm");
        assert_eq!(diagnostic.code,error,"{body}: {diagnostic}");
    }
    let plain="pub fnc read_channel_guest_v1(Channel<int> c): Result<Option<int>,IoError> { val result << c; return result; } pub fnc main(): int { return 0; }";
    let diagnostic=compile(plain).expect_err("plain callable cannot perform direct suspending receive");
    assert_eq!(diagnostic.code,"E_SUSPENDS_REQUIRED","{diagnostic}");
}
