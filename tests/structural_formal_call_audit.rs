//! Structural ~ formal and argument grammar/ABI audit.
//! Production accepts the *formals* as AST but must not lower them as
//! nominal pointers until an explicit readonly structural adapter exists.
//! The syntax-only call plan never claims to execute values.
use atto_compiler::{compile, lexer::lex, parser::Parser, structural_syntax as syn};
use atto_compiler::ast::Type;

#[test]
fn formal_tilde_alias_is_production_ast_not_erased_into_nominal_type() {
    for spelling in [
        "~Point p",
        "~ Point p",
        "~/* gap */ Point p",
        "p: ~Point",
        "p: ~ Point",
        "p: structural Point",
        "structural Point p",
    ] {
        let source = format!(
            "define class Point as pub int x = 42; end pub fnc f({spelling}): int {{ return p.x; }} pub fnc main(): int {{ return 42; }}"
        );
        let mut parser = Parser::new(lex(&source).unwrap());
        let program = parser.parse_program().unwrap_or_else(|d| panic!("{spelling}: {d}"));
        let p = &program.funcs[0].params[0];
        assert_eq!(p.name, "p");
        assert!(p.structural, "{spelling}");
        assert!(matches!(p.ty, Type::Named(_)));
        let error = compile(&source).expect_err("unverified structural ABI must not create Wasm");
        assert_eq!(error.code, "E_UNSUPPORTED", "{spelling}: {error}");
        assert!(error.message.contains("projected-class ABI"), "{error}");
    }
}

#[test]
fn same_contract_holds_for_routine_and_inline_methods() {
    for source in [
        "define class Point as pub int x = 42; end routine read(~Point p): int { return p.x; } pub fnc main(): int { return 42; }",
        "define class Point as pub int x = 42; end define class Reader as pub read(p: ~Point): int { return p.x; } end pub fnc main(): int { return 42; }",
        "define class Point as pub int x = 42; end define class Reader as pub static fnc read(~ Point p): int { return p.x; } end pub fnc main(): int { return 42; }",
    ] {
        let error = compile(source).expect_err("non-nominal ABI remains fail closed");
        assert_eq!(error.code, "E_UNSUPPORTED", "{source}: {error}");
    }
}

#[test]
fn unsupported_formals_never_miscompile_or_move_actor_ownership() {
    for (formal, expected) in [
        ("~int p", "E_UNSUPPORTED_TYPE"),
        ("p: structural bool", "E_UNSUPPORTED_TYPE"),
        ("~{x: int} p", "E_UNSUPPORTED_TYPE"),
        ("p: ~ {x: int}", "E_UNSUPPORTED_TYPE"),
        ("~Point mut p", "E_BORROW"),
        ("p: ~ Point mut", "E_BORROW"),
        ("~Point p = new Point()", "E_UNSUPPORTED"),
    ] {
        let src=format!(
            "define class Point as pub int x = 42; end pub fnc f({formal}): int {{ return 42; }} pub fnc main(): int {{ return 42; }}"
        );
        let error=compile(&src).expect_err("must reject before guest Wasm");
        assert_eq!(error.code, expected, "{formal}: {error}");
    }
}

#[test]
fn named_args_and_tilde_projections_stay_fail_closed_in_all_call_contexts() {
    for source in [
        "pub fnc f(int p): int { return p; } pub fnc main(): int { return f(p: 42); }",
        "pub fnc f(int p): int { return p; } pub fnc main(): int { return f(p: ~object); }",
        "pub fnc f(int p): int { return p; } pub fnc main(): int { return f(~ object); }",
        "define class P as pub int x=42; end pub fnc f(P p): int { return p.x; } pub fnc main(): int { val p=new P(); return f(~p); }",
        "define class P as pub int x=42; end define class C as pub f(P p): int { return p.x; } end pub fnc main(): int { val c=new C(); val p=new P(); return c.f(p: ~p); }",
    ] {
        let error = compile(source).expect_err("callee argument path must not silently erase ~ or labels");
        assert_eq!(error.code, "E_UNSUPPORTED", "{source}: {error}");
    }
}

#[test]
fn syntax_only_binding_is_resolved_before_structural_admission() {
    let formals = syn::parse_formals("(item: ~Point, scale: int, verbose: bool)").unwrap();
    let args = syn::parse_arguments("(~obj.read(), verbose: true, scale: sum(4, 5))").unwrap();
    assert_eq!(syn::bind_checked_structural(&formals,&args).unwrap(), vec![0, 2, 1]);
    let args = syn::parse_arguments("(item: ~ Point, scale: 42, verbose: true)").unwrap();
    assert_eq!(syn::bind_checked_structural(&formals,&args).unwrap(), vec![0, 1, 2]);
    // An explicit structural argument cannot select a nominal formal.
    let ordinary = syn::parse_formals("(item: Point)").unwrap();
    let bad=syn::parse_arguments("(item: ~obj)").unwrap();
    let e=syn::bind_checked_structural(&ordinary,&bad).unwrap_err();
    assert_eq!(e.code,"E_TYPE");
    assert!(e.pos.line>0 && e.pos.col>0);
    for bad in [
        "(item: ~obj, item: ~obj)",
        "(item: ~obj, 42)",
        "(item: ~obj, scale: 42, scale: 99)",
    ] {
        assert!(syn::parse_arguments(bad).is_err(), "{bad}");
    }
    let typo=syn::parse_arguments("(unknown: ~obj, scale: 42, verbose: true)").unwrap();
    let e=syn::bind_checked_structural(&formals,&typo).unwrap_err();
    assert_eq!(e.code,"E_UNBOUND");
    assert!(e.pos.line>0 && e.pos.col>0);
}

#[test]
fn tilde_operand_is_one_postfix_expression_not_a_magic_cast() {
    for good in [
        "(~point)",
        "(~ point)",
        "(~/*c*/point)",
        "(item: structural point)",
        "(item: ~point.field)",
        "(item: ~point.at(0))",
        "(item: ~point.at(index: 0))",
        "(item: ~point[0])",
    ] {
        let a=syn::parse_arguments(good).unwrap_or_else(|d| panic!("{good}: {d}"));
        assert_eq!(a[0].kind,syn::ArgKind::StructuralProjection, "{good}");
    }
    for bad in [
        "(~)", "(~42)", "(~true)", "(~self)", "(~{})",
        "(~{x: 10})", "(structural {x: 10})", "(~ (thing))",
        "(structural (x: 42))", "(~point + 1)", "(~point.)",
        "(~point[1)", "(~point.at()) +", "(~point.some?)",
    ] {
        assert!(syn::parse_arguments(bad).is_err(), "{bad}");
    }
    // Tilde-prefixed collection constructors have been removed entirely.
    for obsolete in [
        "(a: ~(x: 10))", "(a: ~ (x: 10))",
        "(b: ~(k => 42))", "(b: ~ (k => 42))",
        "(b: ~ /* comment */ (k => 42))",
    ] {
        assert!(syn::parse_arguments(obsolete).is_err(), "{obsolete}");
    }
    assert_eq!(syn::parse_arguments("(a: Record.of(x: 10))").unwrap()[0].kind,
               syn::ArgKind::Ordinary);
    assert_eq!(syn::parse_arguments("(b: Map.of(\"k\", 42))").unwrap()[0].kind,
               syn::ArgKind::Ordinary);
}
