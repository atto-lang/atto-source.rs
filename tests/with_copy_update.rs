//! Candidate with syntax and scoped Record type checks.
//! Parse/type-only: no guest Record copying, allocation or execution is claimed.
use atto_compiler::{
    ast::{ExprKind, Program, Stmt},
    collection_scopes, compile, diag::Diagnostic, lexer, parser::Parser,
};

fn parse(body: &str) -> Result<Program, Diagnostic> {
    let source = format!("fnc main(): int {{\n{body}\nreturn 0;\n}}");
    Parser::new(lexer::lex(&source)?).parse_program()
}
fn scoped(body: &str) -> Result<(), Diagnostic> {
    collection_scopes::analyze(&parse(body)?)
}
fn failure(body: &str, code: &str) {
    let e = scoped(body).expect_err(body);
    assert_eq!(e.code, code, "{body}: {e}");
}
#[test]
fn parses_with_as_distinct_immutable_ast() {
    let p = parse("let original = {a: 1}; let updated = original with {a: 2, b: true};").unwrap();
    let Stmt::Bind {init,..} = &p.funcs[0].body[1] else { panic!("expected binding") };
    let ExprKind::With(base, fields) = &init.kind else { panic!("expected With AST") };
    assert!(matches!(base.kind, ExprKind::Var(ref name) if name == "original"));
    assert_eq!(fields.iter().map(|(name,_)|name.as_str()).collect::<Vec<_>>(),["a","b"]);
}
#[test]
fn retains_original_shape_and_changes_only_result() {
    scoped("let source = {name: \"Alice\", age: 30};
            let updated = source with {age: 31};
            let x: string = source.name;
            let y: int = updated.age;").unwrap();
}
#[test]
fn accepts_added_fields_and_changed_field_type() {
    scoped("let source = {age: 30};
            let next = source with {active: true, age: \"thirty\"};
            let a: bool = next.active;
            let b: string = next.age;
            let original_age: int = source.age;").unwrap();
}
#[test]
fn changed_field_type_fails_wrong_expected_projection() {
    failure("let x = {a: 1};
             let y = x with {a: \"str\"};
             let no: int = y.a;", "E_TYPE");
}
#[test]
fn quoted_utf8_and_empty_keys_are_record_fields() {
    scoped("let x = {\"content-type\": \"a\"};
            let y = x with {\"content-type\": \"b\", \"\": true};
            let a: bool = y[\"\"];").unwrap();
}
#[test]
fn empty_update_keeps_static_shape() {
    scoped("let x = {a: 1}; let y = x with {}; let z: int = y.a;").unwrap();
}
#[test]
fn chained_updates_use_latest_static_shape() {
    scoped("let x = {a: 1};
            let y = x with {b: true} with {a: \"changed\"};
            let a: string = y.a; let b: bool = y.b;").unwrap();
}
#[test]
fn computed_scalar_updates_are_inferred_without_retyping_original() {
    scoped("let x = {age: 30, tag: \"A\"};
            let y = x with {age: x.age + 1, tag: x.tag + \"!\"};
            let before: int = x.age;
            let after: int = y.age;
            let text: string = y.tag;").unwrap();
}
#[test]
fn computed_update_incompatible_context_is_rejected() {
    failure("let x = {age: 30};
             let y = x with {age: x.age eq 30};
             let invalid: int = y.age;", "E_TYPE");
}
#[test]
fn negative_oversized_literal_update_is_rejected() {
    failure("let x = {age: 1};
             let y = x with {age: -9223372036854775809};", "E_INTEGER_RANGE");
}
#[test]
fn duplicates_are_rejected_even_if_quoted_and_bare_are_mixed() {
    for source in [
        "let x = {a: 1}; let y = x with {a: 2, a: 3};",
        "let x = {a: 1}; let y = x with {a: 2, \"a\": 3};",
    ] {
        assert_eq!(parse(source).expect_err(source).code,"E_DUPLICATE");
    }
}
#[test]
fn map_rhs_and_nonbrace_rhs_are_syntax_errors() {
    assert_eq!(parse("let x = {a: 1}; let y = x with {\"a\" => 2};")
        .expect_err("Map RHS").code,"E_WITH_MAP");
    assert_eq!(parse("let x = {a: 1}; let y = x with (2);")
        .expect_err("must use braces").code,"E_SYNTAX");
}
#[test]
fn with_is_reserved_as_a_local_name() {
    assert!(parse("let with = 1;").is_err());
}
#[test]
fn rejects_with_on_scalar_and_map() {
    failure("let n = 1; let y = n with {a: 2};","E_WITH_TYPE");
    failure("let m: Map<string,int> = {\"a\" => 1};
             let y = m with {b: 2};","E_WITH_TYPE");
}
#[test]
fn rejects_open_record_without_closed_shape_proof() {
    let source="fnc modify(r: Record): int {
        let result = r with {flag: true};
        return 0;
    }
    fnc main(): int { return 0; }";
    let p=Parser::new(lexer::lex(source).unwrap()).parse_program().unwrap();
    assert_eq!(collection_scopes::analyze(&p).unwrap_err().code,"E_WITH_OPEN_RECORD");
}
#[test]
fn retained_mutable_children_must_have_copy_share_proof() {
    failure("let source = {items: [1, 2]};
             let x = source with {enabled: true};","E_WITH_OWNERSHIP");
}
#[test]
fn unresolved_override_types_are_not_guessed() {
    failure("let x = {a: 1};
             let y = x with {a: unknown()};","E_WITH_TYPE");
}
#[test]
fn browser_guest_remains_fail_closed() {
    let source="fnc main(): int {
        let x = {a: 1};
        let y = x with {b: 2};
        return 0;
    }";
    let e=compile(source).expect_err("guest collection copy not implemented");
    assert_eq!(e.code,"E_UNSUPPORTED","{e}");
}
