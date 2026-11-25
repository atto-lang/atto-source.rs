//! Security and parity audit for effectful callable signatures and mtd lowering.
//! Test the real parser and real emitted guest Wasm; unsupported types fail closed.
use atto_compiler::{ast::Type, compile, lexer::lex, parser::Parser, target_syntax};

fn parse(src: &str) -> atto_compiler::ast::Program {
    Parser::new(lex(src).expect("lex")).parse_program().expect("parse")
}
fn assert_wasm(src: &str) {
    let wasm = compile(src).unwrap_or_else(|e| panic!("unexpected compile failure: {e}; source: {src}"));
    assert_eq!(&wasm[..8], b"\0asm\x01\0\0\0");
    assert_eq!(wasm, compile(src).unwrap(), "guest byte instability");
}

#[test]
fn upper_case_effect_spelling_requires_at_annotation_marker() {
    for bad in [
        "pub fnc main(): int Throws(Error) { return 42; }",
        "pub fnc main(): int Raises(Error) { return 42; }",
        "pub fnc main(): int Panics(Error) { return 42; }",
        "pub fnc main() Throws(Error) -> int { return 42; }",
        "define class C as m(): int Throws(Error) { return 42; } end pub fnc main(): int { return 42; }",
    ] {
        let diag = compile(bad).expect_err("unmarked uppercase effect must not produce guest");
        assert_eq!(diag.code, "E_SYNTAX", "{bad}: {diag}");
    }
    assert_wasm("@Throws(Error)\npub fnc main(): int { return 42; }");
}

#[test]
fn prefixed_generic_returns_preserve_type_identity() {
    let src = r#"
fnc make_result() throws Error -> Result<int,string>
    return Ok(42);
end
Option<int>• optional() throws Error begin
    return None;
end
Option<Result<int,string>>• nested() throws Error begin
    return Some(Ok(42));
end
pub fnc main(): int { return 42; }
"#;
    let program = parse(src);
    assert_eq!(program.funcs[0].ret, Type::ResultIntStr);
    assert_eq!(program.funcs[1].ret, Type::OptionInt);
    assert_eq!(program.funcs[2].ret, Type::OptionResultIntStr);
    for f in &program.funcs[..3] { assert_eq!(f.failure_effects.mask(), 1); }
    assert_wasm(src);
}

#[test]
fn class_mtd_annotations_and_generic_return_types_survive_lowering() {
    let src = r#"
define class Box as
end
@Throws(Error)
@Raises(Error)
pub mtd (self: Box) Option<Result<int,string>>• nested()
    begin
        return Some(Ok(42));
    end
@Throws(Error)
pub mtd (self: Box) result() raises Error -> Result<int,string>
    return Ok(42);
end
pub fnc main(): int { return 42; }
"#;
    let headers = target_syntax::parse_target_declarations(&lex(src).unwrap()).unwrap();
    assert_eq!(headers.receivers.len(), 2);
    assert_eq!(headers.receivers[0].return_type, "Option<Result<int,string>>");
    assert_eq!(headers.receivers[1].return_type, "Result<int,string>");
    let lowered = target_syntax::lower_class_receivers(&lex(src).unwrap(), &headers).unwrap();
    let program = Parser::new(lowered).parse_program().unwrap();
    assert_eq!(program.classes[0].methods.len(), 2);
    assert_eq!(program.classes[0].methods[0].failure_effects.mask(), 3);
    assert_eq!(program.classes[0].methods[1].failure_effects.mask(), 3);
    assert_eq!(program.classes[0].methods[0].ret, Type::OptionResultIntStr);
    assert_eq!(program.classes[0].methods[1].ret, Type::ResultIntStr);
    assert_wasm(src);
}

#[test]
fn explicit_empty_begin_end_is_consistent_with_or_without_mtd() {
    assert_wasm("fnc noop(): void begin end pub fnc main(): int { return 42; }");
    let src = r#"
define class Empty as
end
mtd (self: Empty) noop(): void begin end
fnc another_noop(): void begin end
pub fnc main(): int { return 42; }
"#;
    assert_wasm(src);
}

#[test]
fn invalid_effect_annotations_do_not_disappear_during_mtd_relocation() {
    let suffix = "pub fnc main(): int { return 42; }";
    for (fragment, expected) in [
        ("@Throws(Error) mtd (self: C) value(): int throws Error { return 42; }", "E_EFFECT_TYPE"),
        ("@Throws(int) mtd (self: C) value(): int { return 42; }", "E_EFFECT_TYPE"),
        ("@Throws(IOError) mtd (self: C) value(): int { return 42; }", "E_UNSUPPORTED_TYPE"),
        ("@Throws(Error) @Throws(Error) mtd (self: C) value(): int { return 42; }", "E_EFFECT_TYPE"),
    ] {
        let src = format!("define class C as end\n{fragment}\n{suffix}");
        let err = compile(&src).expect_err("invalid relocated effect must fail closed");
        assert_eq!(err.code, expected, "{src}: {err}");
    }
}

#[test]
fn invalid_nested_generic_headers_fail_before_guest_output() {
    for bad in [
        "pub Option<Result<int,>>• main(): int { return 42; }",
        "define class C as end mtd (self: C) Option<Result<int,>>• value() { return None; } pub fnc main(): int { return 42; }",
    ] {
        assert!(compile(bad).is_err(), "invalid generic return accepted: {bad}");
    }
}

#[test]
fn annotated_function_in_mtd_unit_keeps_effects() {
    let src = r#"
define class C as end
mtd (self: C) value(): int { return 42; }
@Throws(Error)
fnc helper(): int { return 42; }
pub fnc main(): int { return helper(); }
"#;
    assert_wasm(src);
}
