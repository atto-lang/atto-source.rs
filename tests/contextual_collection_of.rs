//! Contextual factory syntax is not a collection allocation capability.
use atto_compiler::{ast::{ExprKind, Stmt}, lexer, parser::Parser, compile};

#[test]
fn factory_of_is_contextual_and_preserves_receiver_and_arguments() {
    for name in ["Map", "Set", "Record"] {
        let src = format!("fnc main(): int {{ let x = {name}.of(1, 2); return 0; }}");
        let p = Parser::new(lexer::lex(&src).unwrap()).parse_program().expect("factory syntax");
        let Stmt::Bind { init, .. } = &p.funcs[0].body[0] else { panic!("binding"); };
        let ExprKind::MethodCall(receiver, method, args) = &init.kind else { panic!("method call"); };
        assert!(matches!(&receiver.kind, ExprKind::Var(n) if n == name));
        assert_eq!(method, "of");
        assert_eq!(args.len(), 2);
        // The current executable checker does not implement these factories.
        // No successful compilation or fabricated collection is acceptable.
        let err = compile(&src).expect_err("no generic collection lowering");
        assert_ne!(err.code, "E_SYNTAX", "{name}: {err}");
    }
}

#[test]
fn of_is_not_a_general_member_or_bare_field_name() {
    for expr in ["x.of(1)", "Map.of", "Set.of", "Record.of", "of(1)"] {
        let src = format!("fnc main(): int {{ let x = {expr}; return 0; }}");
        let err = Parser::new(lexer::lex(&src).unwrap()).parse_program().expect_err(expr);
        assert_eq!(err.code, "E_SYNTAX", "{expr}: {err}");
    }
}
