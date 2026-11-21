//! Candidate Rust parity with Java's inline receiver/source-arity contract.
//! Parser-only proof. The current Rust Method AST has no mutable self loan
//! certificate; never treat this as actor or native-guest runtime parity.
use atto_compiler::{ast::{ExprKind, Stmt}, lexer, parser::Parser};

fn parse(source: &str) -> atto_compiler::ast::Program {
    Parser::new(lexer::lex(source).expect("lex"))
        .parse_program().expect("parse")
}

#[test]
fn leading_readonly_self_has_zero_value_slots_and_does_not_change_arity() {
    let p = parse(r#"
        define class Implicit as
            value(int n): int { return n; }
            no_args(): int { return 0; }
        end
        define class Explicit as
            value(self, int n): int { return n; }
            no_args(self): int { return 0; }
        end
        pub fnc main(): int { return 0; }
    "#);
    assert_eq!(p.classes.len(), 2);
    let implicit = &p.classes[0].methods;
    let explicit = &p.classes[1].methods;
    assert_eq!(implicit.len(), explicit.len());
    for (a, b) in implicit.iter().zip(explicit) {
        assert_eq!(a.name, b.name);
        assert_eq!(a.params.len(), b.params.len(),
            "explicit receiver cannot add a value parameter");
        assert_eq!(a.params.iter().map(|p| &p.name).collect::<Vec<_>>(),
                   b.params.iter().map(|p| &p.name).collect::<Vec<_>>());
    }
    assert_eq!(explicit[0].params.len(), 1);
    assert!(explicit[1].params.is_empty());
}

#[test]
fn explicit_self_at_call_site_is_still_a_supplied_argument() {
    let p = parse(r#"
        define class Counter as
            value(self, int n): int { return n; }
            call(): int { return self.value(self, 5); }
        end
        pub fnc main(): int { return 0; }
    "#);
    let body = &p.classes[0].methods[1].body;
    let Stmt::Return(Some(expr), _) = &body[0] else { panic!("missing return") };
    let ExprKind::MethodCall(_, name, supplied) = &expr.kind else {
        panic!("explicit receiver call was not preserved")
    };
    assert_eq!(name, "value");
    assert_eq!(supplied.len(), 2,
        "self inside call parentheses is not an implicit receiver");
}

#[test]
fn unsupported_receiver_mutability_and_malformed_declarations_fail_closed() {
    for (method, expected) in [
        ("static fnc bad(self, int n): int { return n; }", "E_UNSUPPORTED"),
        ("bad(mut self, int n): int { return n; }", "E_UNSUPPORTED"),
        ("bad(self,): int { return 0; }", "E_SYNTAX"),
    ] {
        let source = format!(
            "define class Counter as {method} end pub fnc main(): int {{ return 0; }}"
        );
        let diagnostic = Parser::new(lexer::lex(&source).unwrap())
            .parse_program().expect_err("invalid receiver must be rejected");
        assert_eq!(diagnostic.code, expected, "{method}: {diagnostic}");
    }
}
