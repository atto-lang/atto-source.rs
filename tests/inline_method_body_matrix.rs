//! Accepted inline-method spelling matrix. Parser assertions deliberately
//! do not pretend that out-of-line receiver dispatch is executable.
//! Named non-void callable cases return explicitly; legacy implicit tails
//! remain a separately versioned grammar concern, not an mtd exception.
use atto_compiler::{lexer::lex, parser::Parser, ast::Stmt};

#[test]
fn inline_methods_accept_five_body_styles_with_canonical_mtd() {
    let source = r#"
        define class Calculator as
            fnc colon_braces(): int { return 5; }
            mtd colon_arrow(): int -> { return 5; }
            arrow_braces() -> int { return 5; }
            arrow_end() -> int
                let int x = 3;
                return x + 2;
            end
            arrow_delimited() -> int ::
                let int x = 3;
                return x + 2;
            end
            static fnc static_braces(): int { return 5; }
        end
        pub fnc main(): int { return 0; }
    "#;
    let program = Parser::new(lex(source).unwrap()).parse_program().unwrap();
    assert_eq!(program.classes.len(), 1);
    let methods = &program.classes[0].methods;
    assert_eq!(methods.len(), 6);
    for method in methods {
        assert_eq!(method.ret, atto_compiler::ast::Type::Int);
        assert!(matches!(method.body.last(), Some(Stmt::Return(Some(_), _))));
    }
    assert!(!methods[0].is_static);
    assert!(methods[5].is_static);
}

#[test]
fn inline_actor_mtd_methods_share_the_five_body_styles() {
    let source = r#"
        define actor Worker as
            fnc colon_braces(): int { return 5; }
            mtd colon_arrow(): int -> { return 5; }
            arrow_braces() -> int { return 5; }
            arrow_end() -> int
                let int x = 3;
                return x + 2;
            end
            arrow_delimited() -> int ::
                let int x = 3;
                return x + 2;
            end
        end
        pub fnc main(): int { return 0; }
    "#;
    let program = Parser::new(lex(source).unwrap()).parse_program().unwrap();
    assert_eq!(program.actors.len(), 1);
    let methods = &program.actors[0].methods;
    assert_eq!(methods.len(), 5);
    for method in methods {
        assert!(matches!(method.body.last(), Some(Stmt::Return(Some(_), _))));
    }
}

#[test]
fn duplicate_arity_is_not_resolved_by_receiver_style() {
    let source = r#"
        define class Calculator as
            fnc get() -> int :: return 5; end
            mtd get() -> int { return 6; }
        end
        pub fnc main(): int { return 0; }
    "#;
    let program = Parser::new(lex(source).unwrap()).parse_program().unwrap();
    assert_eq!(program.classes[0].methods.len(), 2);
    // The semantic checker (not source spelling) must reject duplicate owners.
    let error = atto_compiler::compile(source).unwrap_err();
    assert_eq!(error.code, "E_DUPLICATE");
}

#[test]
fn empty_erlang_body_is_not_a_valid_value_or_void_method() {
    for header in ["empty() -> int :: end", "empty() -> void end"] {
        let src = format!("define class C as {header} end pub fnc main(): int {{ return 0; }}");
        let d = Parser::new(lex(&src).unwrap()).parse_program().unwrap_err();
        assert_eq!(d.code, "E_SYNTAX", "{src}: {d}");
    }
}

#[test]
fn inline_mtd_compiles_as_instance_call_without_mutability_or_arity_changes() {
    let source = r#"
        define class Calculator as
            final int seed: 3;
            pub mtd answer() : int { return self.seed + 39; }
            pub mtd legacy() : int { return self.seed + 1; }
            static fnc static_answer(): int { return 42; }
        end
        pub fnc main(): int {
            val calc = new Calculator();
            return calc.answer();
        }
    "#;
    let program = Parser::new(lex(source).unwrap()).parse_program().unwrap();
    assert_eq!(program.classes[0].methods.len(), 3);
    assert!(!program.classes[0].methods[0].is_static);
    assert!(!program.classes[0].methods[1].is_static);
    assert!(program.classes[0].methods[2].is_static);
    assert!(atto_compiler::compile(source).is_ok(), "inline mtd must lower through ordinary instance dispatch");
}

#[test]
fn mtd_does_not_bypass_static_fnc_or_package_visibility_gates() {
    for (source, code) in [
        ("define class C as static mtd f(): int { return 42; } end pub fnc main(): int { return 0; }", "E_SYNTAX"),
        ("define class C as pub[pkg] mtd f(): int { return 42; } end pub fnc main(): int { return 0; }", "E_UNSUPPORTED"),
    ] {
        let diagnostic = atto_compiler::compile(source).expect_err("unsupported mtd form must fail closed");
        assert_eq!(diagnostic.code, code, "{source}: {diagnostic}");
    }
}

#[test]
fn inline_mtd_state_container_retains_critical_lease_and_reserved_hook_guards() {
    // A method prefix must not turn protected state into ordinary class fields.
    let source = r#"
        define state container Flag as
            state { let bool enabled: true; }
            pub mtd read_enabled(): bool {
                return handle_state(critical-read |s| -> s.enabled);
            }
        end
        pub fnc main(): int { return 0; }
    "#;
    let program = Parser::new(lex(source).unwrap()).parse_program().unwrap();
    assert!(program.classes[0].state_container);
    assert_eq!(program.classes[0].methods.len(), 1);
    assert_eq!(program.classes[0].methods[0].name, "read_enabled");
    let wasm = atto_compiler::compile(source).expect("mtd state method must retain ordinary state-container lowering");
    assert!(wasm.starts_with(&[0, 97, 115, 109, 1, 0, 0, 0]));

    let override_source = r#"
        define state container Flag as
            state { let bool enabled: true; }
            mtd handle_state(): bool { return true; }
        end
        pub fnc main(): int { return 0; }
    "#;
    let diagnostic = Parser::new(lex(override_source).unwrap())
        .parse_program().expect_err("compiler-owned handle_state cannot be shadowed through mtd");
    assert_eq!(diagnostic.code, "E_STATE");
}

#[test]
fn explicit_inline_actor_mtd_still_satisfies_nominal_method_contracts() {
    let source = r#"
        define interface IPing as ping(): int; end
        define actor Worker implements IPing as
            pub mtd ping(): int { return 42; }
            receive(ActorMail<int> mail): void {
                stdio.println(self.ping());
            }
        end
        pub fnc main(): int { return 0; }
    "#;
    // The raw subset parser runs after nominal interface lowering in the
    // production pipeline. Exercise its inline mtd AST with the same actor
    // body and exercise the full contract through compile() below.
    let raw_actor = source.replace("define interface IPing as ping(): int; end", "")
        .replace("define actor Worker implements IPing as", "define actor Worker as");
    let program = Parser::new(lex(&raw_actor).unwrap()).parse_program().unwrap();
    assert_eq!(program.actors[0].methods[0].name, "ping");
    assert!(atto_compiler::compile(source).is_ok(),
        "mtd must lower exactly like an inline actor method, without skipping interface checks");
}

#[test]
fn contextual_mtd_and_method_remain_callable_names() {
    // Java and JS support the method spellings as ordinary names when followed
    // immediately by '('. A declaration marker instead precedes a second name.
    let source = r#"
        define class Names as
            pub mtd(): int { return 10; }
            pub method(): int { return 20; }
            pub mtd named(): int { return 12; }
        end
        pub fnc main(): int {
            val names = new Names();
            return names.mtd() + names.method() + names.named();
        }
    "#;
    let program = Parser::new(lex(source).unwrap()).parse_program().unwrap();
    let names: Vec<&str> = program.classes[0].methods
        .iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names, ["mtd", "method", "named"]);
    let wasm = atto_compiler::compile(source).expect("contextual method names must compile");
    assert!(wasm.starts_with(&[0, 97, 115, 109, 1, 0, 0, 0]));
}

#[test]
fn state_container_mtd_name_is_not_greedily_consumed_as_marker() {
    let source = r#"
        define state container Flag as
            state { let bool enabled: true; }
            pub mtd(): bool {
                return handle_state(critical-read |s| -> s.enabled);
            }
        end
        pub fnc main(): int { return 0; }
    "#;
    let program = Parser::new(lex(source).unwrap()).parse_program().unwrap();
    assert_eq!(program.classes[0].methods[0].name, "mtd");
    assert!(atto_compiler::compile(source).is_ok());
}

#[test]
fn consecutive_method_prefix_markers_are_not_accepted() {
    for invalid in ["mtd method foo()", "method mtd foo()", "fnc mtd foo()", "method foo()", "method method()"] {
        let source = format!(
            "define class C as {invalid}: int {{ return 42; }} end pub fnc main(): int {{ return 0; }}"
        );
        assert!(atto_compiler::compile(&source).is_err(), "duplicate marker admitted: {source}");
    }
}

#[test]
fn bare_end_body_call_named_mtd_is_not_reclassified_as_external_receiver() {
    let source = r#"
        pub fnc mtd(): int { return 42; }
        pub fnc main() -> int ::
            return mtd();
        end
    "#;
    let guest = atto_compiler::compile(source)
        .expect("the lexer word mtd is a contextual callable name, not always a receiver marker");
    assert!(guest.starts_with(&[0, 97, 115, 109, 1, 0, 0, 0]));
}

#[test]
fn external_class_receiver_remains_on_checked_target_path() {
    // This was already executable before inline mtd admission; the new
    // mode-selection logic must not turn it into an unchecked bare function.
    const SOURCE: &str = include_str!("fixtures/pass/173-class-mtd-executable.atto");
    let compiled = atto_compiler::compile(SOURCE)
        .expect("verified immutable out-of-line mtd still compiles");
    assert!(compiled.starts_with(&[0, 97, 115, 109, 1, 0, 0, 0]));
}
