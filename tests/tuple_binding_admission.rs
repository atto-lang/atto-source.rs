use atto_compiler::lexer::lex;
use atto_compiler::tuple_binding_admission::{inspect, BindingMode};

fn parse(s: &str) -> Vec<atto_compiler::tuple_binding_admission::TupleBinding> {
    inspect(&lex(s).unwrap()).unwrap()
}
fn reject(s: &str, code: &str) {
    let e = inspect(&lex(s).unwrap()).unwrap_err();
    assert_eq!(e.code, code, "{}", e.message);
}

#[test]
fn outer_let_and_mut_element_are_independent() {
    let b = parse("let (a, r, mut j) as foo = fixtures();");
    assert_eq!(b.len(), 1);
    assert_eq!(b[0].members.len(), 3);
    assert_eq!(b[0].members[2].mode, BindingMode::Let);
    assert!(b[0].members[2].mutable_referent);
    assert_eq!(b[0].alias.as_ref().unwrap().mode, BindingMode::Let);
    assert_eq!(b[0].alias.as_ref().unwrap().name, "foo");
}

#[test]
fn root_binding_kind_can_be_overridden() {
    for (text, expected) in [
        ("let (a, r, mut j) as val foo = fixtures();", BindingMode::Val),
        ("let (a, r, mut j) as const foo = fixtures();", BindingMode::Const),
        ("let (a, r, mut j) as let foo = fixtures();", BindingMode::Let),
        ("val (a, r, mut j) as foo = fixtures();", BindingMode::Val),
        ("const (a, r, mut j) as foo = (1,2,3);", BindingMode::Const),
    ] {
        let b = parse(text);
        assert_eq!(b[0].alias.as_ref().unwrap().mode, expected, "{text}");
    }
}

#[test]
fn each_element_can_override_outer_kind() {
    let b = parse("let (a: int, val r: int, val mut j: int) = fixtures();");
    assert_eq!(b[0].members[0].mode, BindingMode::Let);
    assert_eq!(b[0].members[1].mode, BindingMode::Val);
    assert_eq!(b[0].members[2].mode, BindingMode::Val);
    assert!(b[0].members[2].mutable_referent);
}

#[test]
fn fully_explicit_tuple_binders_need_explicit_root_kind() {
    let b = parse("(let a, val r, let mut j) as const foo = (1,2,3);");
    assert_eq!(b[0].alias.as_ref().unwrap().mode, BindingMode::Const);
    reject("(let a, val r, let mut j) as foo = fixtures();", "E_SYNTAX");
    // Without an outer mode or explicit per-element modes this is not a
    // tuple binding header; the full compiler must reject it normally.
    assert!(parse("(a, r, mut j) = fixtures();").is_empty());
}

#[test]
fn singleton_empty_and_discard_have_precise_shapes() {
    assert!(parse("let () = ();")[0].members.is_empty());
    assert_eq!(parse("let (only,) = (42,);")[0].members.len(), 1);
    assert_eq!(parse("let (a, _, j) = fixtures();")[0].members[1].name, None);
    reject("let (only) = (42,);", "E_SYNTAX");
    reject("let (mut _, r) = fixtures();", "E_SYNTAX");
}

#[test]
fn all_binders_and_root_share_single_scope() {
    reject("let (a, a, j) = fixtures();", "E_DUPLICATE");
    reject("let (a, r, j) as a = fixtures();", "E_DUPLICATE");
    reject("let (a, r, j) as const j = fixtures();", "E_DUPLICATE");
}

#[test]
fn malformed_header_and_nested_shapes_fail_closed() {
    reject("let (a, r, mut j = fixtures();", "E_SYNTAX");
    reject("let (a, (r, j)) = fixtures();", "E_SYNTAX");
    reject("let (a, r, j) as foo fixtures();", "E_SYNTAX");
    reject("let (a, r,, j) = fixtures();", "E_SYNTAX");
}

#[test]
fn lexical_comments_and_strings_are_not_binding_declarations() {
    let s = r#"// let (a, mut b) as foo = x;
pub fnc main(): int {
    let text = "let (a, mut b) as foo = x;";
    return 7;
}"#;
    assert!(parse(s).is_empty());
}

#[test]
fn compiler_never_exposes_unproven_tuple_runtime() {
    for s in [
        "let (a,r,mut j) as foo = fixtures();",
        "let (a,r,mut j) as val foo = fixtures();",
        "let (a,r,mut j) as const foo = fixtures();",
        "(let a,val r,let mut j) as const foo = fixtures();",
    ] {
        let program = format!("pub fnc main(): int {{ {s} return 0; }}");
        let err = atto_compiler::compile(&program).unwrap_err();
        assert_eq!(err.code, "E_UNSUPPORTED", "{s}: {}", err.message);
    }
}
