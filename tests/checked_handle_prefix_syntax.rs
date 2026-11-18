//! The canonical checked-handle prefixes and their legacy rt spellings
//! must produce identical parse trees. Until the backend has a verified
//! checked-handle MIR/runtime, ALL of them must fail closed before codegen.
use atto_compiler::{
    ast::{Expr, ExprKind, OwnershipOp, Stmt},
    lexer, parser::Parser,
};

fn expr(source: &str) -> Expr {
    let src = format!("pub fnc main(): int {{ val result = {source}; return 0; }}");
    let p = Parser::new(lexer::lex(&src).expect("lex"))
        .parse_program().expect("parse canonical checked-handle syntax");
    let Stmt::Bind { init, .. } = &p.funcs[0].body[0] else {
        panic!("expected binding");
    };
    init.clone()
}

fn checked_op(source: &str, expected: OwnershipOp) -> ExprKind {
    let actual = expr(source);
    let ExprKind::Ownership(op, mutable, inner) = actual.kind else {
        panic!("expected an ownership or checked-handle AST node: {source}");
    };
    assert_eq!(op, expected, "{source}");
    assert!(!mutable, "{source}: checked handles cannot be mutable borrows");
    inner.kind
}

#[test]
fn canonical_and_legacy_handle_prefixes_have_the_same_ast() {
    for (modern, compact, grouped, legacy, action) in [
        ("ref! h", "ref!h", "ref!(h)", "rt ref h", OwnershipOp::Ref),
        ("deref! h", "deref!h", "deref!(h)", "rt deref h", OwnershipOp::Deref),
        ("unref! h", "unref!h", "unref!(h)", "rt unref h", OwnershipOp::Unref),
    ] {
        let expected = format!("{:?}", checked_op(modern, action));
        assert_eq!(expected, format!("{:?}", checked_op(compact, action)));
        assert_eq!(expected, format!("{:?}", checked_op(grouped, action)));
        assert_eq!(expected, format!("{:?}", checked_op(legacy, action)));
        assert_eq!(expected, "Var(\"h\")");
    }
}

#[test]
fn postfix_operands_preserve_exact_structure_not_a_function_call() {
    let member = checked_op("deref! h.value", OwnershipOp::Deref);
    assert!(matches!(member,
        ExprKind::Member(base, field) if field == "value"
            && matches!(base.kind, ExprKind::Var(ref name) if name == "h")));

    let nested = checked_op("ref! (deref! h)", OwnershipOp::Ref);
    assert!(matches!(nested, ExprKind::Ownership(OwnershipOp::Deref, false, _)));
}

#[test]
fn unsupported_handle_abi_is_rejected_for_bang_and_legacy_spellings() {
    for form in [
        "ref! h", "ref!h", "ref!(h)", "rt ref h",
        "deref! h", "deref!h", "deref!(h)", "rt deref h",
        "unref! h", "unref!h", "unref!(h)", "rt unref h",
    ] {
        let src = format!("pub fnc main(): int {{ val h = 42; val x = {form}; return 0; }}");
        let e = atto_compiler::compile(&src).unwrap_err();
        assert_eq!(e.code, "E_UNSUPPORTED_HANDLE", "{form}: {e:?}");
    }
}

#[test]
fn ref_syntax_rejects_separated_marker_and_mut_qualifier() {
    for illegal in ["ref !h", "deref !h", "unref !h",
                    "ref! mut h", "deref! mut h", "unref! mut h",
                    "rt ref mut h", "rt deref mut h", "rt unref mut h"] {
        let src = format!("pub fnc main(): int {{ val h = 42; val x = {illegal}; return 0; }}");
        let parsed = Parser::new(lexer::lex(&src).expect("lex")).parse_program();
        assert!(parsed.is_err(), "invalid checked-handle syntax accepted: {illegal}");
    }
}

#[test]
fn similar_identifiers_without_attached_bang_remain_ordinary_calls() {
    let src = r#"
        fnc ref(int h): int { return h; }
        fnc deref(int h): int { return h; }
        fnc unref(int h): int { return h; }
        pub fnc main(): int { return ref(deref(unref(42))); }
    "#;
    let wasm = atto_compiler::compile(src).expect("ordinary identifiers are not handle prefixes");
    assert!(wasm.starts_with(b"\0asm"));
}
