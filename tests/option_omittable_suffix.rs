//! Grammar position, ABI and collision regressions for Option<T> type suffix
//! versus omittable call-site formals. These are production parser/checker tests.
use atto_compiler::{compile, lexer::lex, parser::Parser, ast::Type};
use atto_compiler::target_syntax::parse_target_declarations;

fn parse(source: &str) -> atto_compiler::ast::Program {
    Parser::new(lex(source).unwrap()).parse_program().unwrap()
}
fn pass(source: &str) {
    let bytes = compile(source).unwrap();
    assert_eq!(&bytes[..8], &[0, 97, 115, 109, 1, 0, 0, 0]);
    assert_eq!(bytes, compile(source).unwrap(), "deterministic Wasm");
}
fn fails(source: &str, code: &str) {
    let d = compile(source).unwrap_err();
    assert_eq!(d.code, code, "{source}: {d}");
}

#[test]
fn type_suffix_and_formal_suffix_are_independent_ast_slots() {
    let p = parse(r#"
        fnc a(int? x?): int? { return None; }
        routine b(x?: int?): Option<int> { return None; }
        fnc c(int? x): Option<int> { return x; }
        pub fnc main(): int { return 0; }
    "#);
    for i in 0..2 {
        assert_eq!(p.funcs[i].params[0].ty, Type::OptionInt);
        assert!(p.funcs[i].params[0].omittable);
        assert_eq!(p.funcs[i].ret, Type::OptionInt);
    }
    assert_eq!(p.funcs[2].params[0].ty, Type::OptionInt);
    assert!(!p.funcs[2].params[0].omittable);
    assert_eq!(p.funcs[2].ret, Type::OptionInt);
}

#[test]
fn fnc_and_routine_inject_real_typed_none_without_changing_abi() {
    pass(r#"
        fnc first(int? x?): int { return 20; }
        routine second(value?: int?): int { return 22; }
        pub fnc main(): int { return first() + second(); }
    "#);
    pass(r#"
        fnc first(int? x?): int { return 20; }
        routine second(value?: int?): int { return 22; }
        pub fnc main(): int { return first(None) + second(Some(1)); }
    "#);
    pass(r#"
        fnc f(int n, int? a?, int? b?): int { return n; }
        pub fnc main(): int { return f(42); }
    "#);
}

#[test]
fn return_and_nested_result_sugar_is_canonical() {
    let p = parse(r#"
        fnc a(): Result<int,string>? { return None; }
        fnc b(): Option<Result<int,string>> { return None; }
        pub fnc main(): int { return 0; }
    "#);
    assert_eq!(p.funcs[0].ret, Type::OptionResultIntStr);
    assert_eq!(p.funcs[1].ret, Type::OptionResultIntStr);
    pass(r#"
        fnc a(): Result<int,string>? { return None; }
        pub fnc main(): int { return 42; }
    "#);
}

#[test]
fn inline_and_out_of_line_class_methods_accept_omission() {
    pass(r#"
        define class Foo as
            pub run(int? x?): int { return 42; }
        end
        pub fnc main(): int { val f = new Foo(); return f.run(); }
    "#);
    pass(r#"
        define class Foo as end
        pub mtd (Foo self) run(int? x?): int { return 42; }
        pub fnc main(): int { val f = new Foo(); return f.run(); }
    "#);
}

#[test]
fn target_actor_mtd_preserves_proposed_syntax_but_fails_closed_on_unlinked_option_abi() {
    let src = r#"
        declare actor Foo;
        mtd (Foo self) run(Foo? x?): void { }
    "#;
    let t = parse_target_declarations(&lex(src).unwrap()).unwrap();
    assert_eq!(t.receivers.len(), 1);
    assert_eq!(t.receivers[0].params[0].type_name, "Option<Foo>");
    assert!(t.receivers[0].params[0].omittable);
    fails(src, "E_UNSUPPORTED_TYPE");
}

#[test]
fn no_overload_ambiguity_is_permitted_at_declaration_time() {
    fails(r#"
        fnc pick(int x): int { return x; }
        fnc pick(int x, int? extra?): int { return x; }
        pub fnc main(): int { return pick(1); }
    "#, "E_AMBIGUOUS");
    fails(r#"
        fnc pick(int? x?): int { return 7; }
        fnc pick(int? x, int? y?): int { return 8; }
        pub fnc main(): int { return pick(None); }
    "#, "E_AMBIGUOUS");
}

#[test]
fn missing_required_and_present_option_arguments_stay_distinct() {
    fails(r#"
        fnc f(int? x): int { return 4; }
        pub fnc main(): int { return f(); }
    "#, "E_ARITY");
    fails(r#"
        fnc f(int? x?): int { return 4; }
        pub fnc main(): int { return f(1); }
    "#, "E_TYPE");
    fails(r#"
        fnc f(int? x?): int { return 4; }
        pub fnc main(): int { return f(x: None); }
    "#, "E_UNSUPPORTED");
}

#[test]
fn invalid_marker_placements_reject_without_ternary_or_propagation_fallback() {
    fails("fnc f(int x?): int { return 1; } pub fnc main(): int { return 0; }",
        "E_OMITTABLE_DEFAULT");
    fails("fnc f(int? x?, int n): int { return n; } pub fnc main(): int { return 0; }",
        "E_OMITTABLE_ORDER");
    fails("fnc f(int?? x): int { return 1; } pub fnc main(): int { return 0; }",
        "E_UNSUPPORTED");
    fails("fnc f(int? x??): int { return 1; } pub fnc main(): int { return 0; }",
        "E_SYNTAX");
    fails("fnc f(void? x): int { return 1; } pub fnc main(): int { return 0; }",
        "E_UNSUPPORTED_TYPE");
    fails("fnc f(Point? x?): int { return 1; } pub fnc main(): int { return 0; }",
        "E_UNSUPPORTED_TYPE");
}

#[test]
fn unused_match_only_adt_functions_still_allocate_linear_memory_for_loads() {
    // The heap decision must not rely on a reachable Some/None constructor.
    // Without ADT-read admission this used to produce structurally invalid Wasm.
    let src = r#"
        fnc inspect(int? x): int {
            match x over
                on None -> { return 42; }
                on Some(n) -> { return n; }
            end
        }
        pub fnc main(): int { return (42); }
    "#;
    let typed = atto_compiler::check::check(&parse(src)).unwrap();
    assert!(typed.heap, "an uncalled ADT match still emits i32.load and needs memory");
    pass(src);
}
