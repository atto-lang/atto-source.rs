//! Parser/module admission for explicit `std.assert` namespace imports.
//! This is **not** proof that the package linker, typed AssertionError or
//! optimized compiled std.assert guest execution is implemented.
use atto_compiler::{compile, lexer::lex, parser::Parser, target_syntax::parse_target_declarations};
use atto_compiler::ast::{ExprKind, Stmt};

fn parse(src: &str) -> atto_compiler::ast::Program {
    Parser::new(lex(src).expect("lexer")).parse_program().expect("parser")
}

#[test]
fn canonical_explicit_module_import_is_recognized() {
    let headers = parse_target_declarations(&lex(
        "import module assert from pkg \"std.assert\"; pub fnc main(): int { return 0; }"
    ).unwrap()).expect("typed module namespace import header must parse");
    assert_eq!(headers.imports, ["assert"]);
    let compat = parse_target_declarations(&lex(
        "import * as assert from pkg \"std.assert\"; pub fnc main(): int { return 0; }"
    ).unwrap()).expect("star-as module namespace import header must parse");
    assert_eq!(compat.imports, ["assert"]);
}

#[test]
fn qualified_boolean_word_members_have_an_ast_not_bare_keywords() {
    let program = parse("pub fnc main(): int { assert.true(true); assert.false(false); return 0; }");
    let main = &program.funcs[0];
    assert_eq!(main.body.len(), 3);
    for (stmt, member, flag) in [(&main.body[0], "true", true), (&main.body[1], "false", false)] {
        match stmt {
            Stmt::Expr(e) => match &e.kind {
                ExprKind::MethodCall(base, actual_member, args) => {
                    assert_eq!(actual_member, member);
                    assert!(matches!(&base.kind, ExprKind::Var(name) if name == "assert"));
                    assert_eq!(args.len(), 1);
                    assert!(matches!(&args[0].kind, ExprKind::Bool(value) if *value == flag));
                }
                other => panic!("not a qualified member call: {other:?}"),
            },
            other => panic!("not an expression: {other:?}"),
        }
    }
}

#[test]
fn reserved_words_stay_reserved_in_bare_and_unrelated_member_positions() {
    let bare = Parser::new(lex("fnc true(): int { return 1; }").unwrap())
        .parse_program().expect_err("bare true is reserved");
    assert_eq!(bare.code, "E_RESERVED");
    let keyword = Parser::new(lex("pub fnc main(): int { assert.if(true); return 0; }").unwrap())
        .parse_program().expect_err("control-flow keyword must not become a member");
    assert_eq!(keyword.code, "E_SYNTAX");
}

#[test]
fn unlinked_package_import_must_fail_without_emitting_wasm() {
    let src = "import module assert from pkg \"std.assert\"; pub fnc main(): int { assert.true(true); return 0; }";
    let error = compile(src).expect_err("a package binding without a real loader cannot run");
    assert!(error.pos.line > 0);
}

#[test]
fn real_local_module_qualified_boolean_members_emit_actual_guest_wasm() {
    // Proves the existing file-local module lowering can bind these member
    // names, but DOES NOT claim that the external std.assert package links.
    let src = r#"
        define module test_module as
            pub fnc true(bool value): int { return 7; }
            pub fnc false(bool value): int { return 8; }
        end
        pub fnc main(): int {
            return test_module.true(true) + test_module.false(false);
        }
    "#;
    let bytes = compile(src).expect("local Atto module is real guest code");
    assert!(bytes.starts_with(b"\0asm\x01\0\0\0"));
    assert_eq!(bytes, compile(src).unwrap(), "same source must emit deterministic guest bytes");
}
