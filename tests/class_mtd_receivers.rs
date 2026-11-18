//! Class receivers are syntax-checked but MUST NOT be linked, typed or
//! dispatched until the class mtd backend is implemented.
use atto_compiler::{compile, lexer::lex};
use atto_compiler::target_syntax::parse_target_declarations;

fn parse(src: &str) -> Result<atto_compiler::target_syntax::TargetHeaders, atto_compiler::Diagnostic> {
    parse_target_declarations(&lex(src).unwrap())
}

#[test]
fn class_receivers_accept_all_five_bodies_and_canonical_aliases() {
    let src = r#"
        define class Calculator as
            fnc inline_square() -> int :: 5 end
        end
        mtd (self: Calculator) a(): int { return 5; }
        mtd (Calculator self) b(): int -> { return 5; }
        mtd (Calculator) c() -> int { return 5; }
        mtd (Calculator _) d() -> int
            const x: int = 3;
            ^ x + 2;
        end
        mtd (mut self: Calculator) e() -> int :: let int x = 3, x + 2 end
        pub fnc main(): int { return 0; }
    "#;
    let p = parse(src).unwrap();
    assert_eq!(p.classes, vec!["Calculator"]);
    assert_eq!(p.class_definitions.len(), 1);
    assert_eq!(p.class_definitions[0].methods.len(), 1);
    assert_eq!(p.receivers.len(), 5);
    assert!(p.receivers.iter().all(|m| m.owner == "Calculator"));
    assert!(p.receivers[4].mutable);
    assert!(p.receivers[..4].iter().all(|m| !m.mutable));
    assert_eq!(compile(src).unwrap_err().code, "E_UNSUPPORTED",
        "recognized class receiver syntax is not executable");
}

#[test]
fn duplicate_inline_and_out_of_line_signatures_fail_closed() {
    let d = parse(r#"
        define class C as
            fnc same() -> int :: 1 end
        end
        mtd (self: C) same() -> int :: 2 end
    "#).unwrap_err();
    assert_eq!(d.code, "E_DUPLICATE");
    let duplicate_alias = parse(r#"
        define class C as end
        mtd (self: C) same() -> int :: 1 end
        mtd (C self) same() -> int :: 2 end
    "#).unwrap_err();
    assert_eq!(duplicate_alias.code, "E_DUPLICATE");
}

#[test]
fn unknown_owner_and_cross_kind_name_collisions_reject() {
    assert_eq!(parse("mtd (self: Missing) same() -> int :: 1 end")
        .unwrap_err().code, "E_UNBOUND");
    let cross = parse(r#"
        declare actor C;
        define class C as end
        mtd (self: C) same() -> int :: 1 end
    "#).unwrap_err();
    assert_eq!(cross.code, "E_DUPLICATE");
}

#[test]
fn nested_end_style_actor_method_does_not_truncate_actor_definition() {
    let src = r#"
        declare actor Worker;
        define actor Worker as
            fnc answer() -> int :: let int x = 3, x + 2 end
        end
        mtd (self: Worker) status() -> int :: 5 end
    "#;
    let p = parse(src).unwrap();
    assert_eq!(p.definitions.len(), 1);
    assert_eq!(p.definitions[0].methods.len(), 1);
    assert_eq!(p.receivers.len(), 1);
    assert_eq!(compile(src).unwrap_err().code, "E_UNSUPPORTED");
}

#[test]
fn actor_mtd_is_an_interface_contract_not_a_duplicate_implementation() {
    let bad = r#"
        use namespace workers;
        declare actor Foo;
        define actor Foo as
            foo(): int { return 1; }
        end
        mtd (Foo self) foo(): bool { return true; }
    "#;
    assert_eq!(compile(bad).unwrap_err().code, "E_SIGNATURE",
        "actor interface mismatches are signatures, not class duplicates");

    let good = r#"
        use namespace workers;
        declare actor Foo;
        define actor Foo as
            foo(): int { return 1; }
        end
        mtd (Foo self) foo(): int { return 1; }
    "#;
    let parsed = parse(good).unwrap();
    assert_eq!(parsed.receivers.len(), 1);
    assert_eq!(compile(good).unwrap_err().code, "E_DUPLICATE",
        "a matching inline/out-of-line actor method is still a duplicate declaration");
}
