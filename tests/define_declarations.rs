//! Top-level `define` declaration admission is intentionally syntax-only:
//! live class/actor/struct semantics are not silently approximated by Wasm.
use atto_compiler::{ast::DeclarationKind, lexer, parser::Parser};

const DECLS: &str = r#"
define class Point as
end
define actor Worker as
end
define struct Pair as
end
define pub module utilities as
end
define interface Comparable as
end
define trait Printable as
end
define supervisor Root as
end
define enum Status as
  Pending
  Ready,
  Failed;
end
pub fnc main(): int { return 42; }
"#;

#[test]
fn top_level_definitions_are_preserved_in_ast() {
    let toks = lexer::lex(DECLS).expect("lex");
    let ast = Parser::new(toks).parse_program().expect("parse");
    assert_eq!(ast.funcs.len(), 1);
    assert_eq!(ast.declarations.len(), 8);
    assert_eq!(ast.declarations[0].kind, DeclarationKind::Class);
    assert_eq!(ast.declarations[1].kind, DeclarationKind::Actor);
    assert_eq!(ast.declarations[2].kind, DeclarationKind::Struct);
    assert_eq!(ast.declarations[3].kind, DeclarationKind::Module);
    assert_eq!(ast.declarations[4].kind, DeclarationKind::Interface);
    assert_eq!(ast.declarations[5].kind, DeclarationKind::Trait);
    assert_eq!(ast.declarations[6].kind, DeclarationKind::Supervisor);
    assert_eq!(ast.declarations[7].kind, DeclarationKind::Enum);
    assert_eq!(ast.declarations[7].variants, ["Pending", "Ready", "Failed"]);
    assert_eq!(ast.declarations[7].name, "Status");
    let bytes = atto_compiler::compile(DECLS).expect("unreferenced declarations are inert");
    assert_eq!(&bytes[..8], b"\0asm\x01\0\0\0");
}

#[test]
fn declaration_admission_never_enables_runtime_features() {
    const MAIN: &str = "pub fnc main(): int { return 0; }";
    let cases = [
        ("define class Point as\n  fnc unsupported(): int { return 0; }\nend\n", "E_UNSUPPORTED"),
        ("define actor Worker as\n  receive(int x) -> void { return; }\nend\n", "E_UNSUPPORTED"),
        ("define struct Point as\n int x: 0\nend\n", "E_UNSUPPORTED"),
        ("define module M as\n  fnc f(): int { return 1; }\nend\n", "E_UNSUPPORTED"),
        ("define enum E as\n X\n X\nend\n", "E_DUPLICATE"),
        ("define enum E as X Y end\n", "E_SYNTAX"),
        ("define enum E as X = 7 end\n", "E_SYNTAX"),
        ("define class P { }\n", "E_SYNTAX"),
        ("define class P as\n", "E_UNSUPPORTED"),
        ("define macro m as end\n", "E_UNSUPPORTED"),
        ("define enum E as X end\ndefine class E as end\n", "E_DUPLICATE"),
        ("define class class as end\n", "E_RESERVED"),
        ("define class Foo<T> as end\n", "E_UNSUPPORTED"),
        ("define actor A as end\nend\n", "E_TOPLEVEL"),
    ];
    for (prefix, code) in cases {
        let src = format!("{prefix}{MAIN}");
        let d = atto_compiler::compile(&src).expect_err(prefix);
        assert_eq!(d.code, code, "{prefix}: {d}");
        assert!(d.pos.line >= 1 && d.pos.col >= 1);
    }
    let missing_end = atto_compiler::compile("define class Missing as\n").unwrap_err();
    assert_eq!(missing_end.code, "E_EOF");
    // A recognized but unused actor declaration cannot authorize spawn.
    let src = "define actor Worker as end\npub fnc main(): int { val x = spawn Worker(); return 0; }";
    assert_eq!(atto_compiler::compile(src).unwrap_err().code, "E_UNSUPPORTED");
    // Fieldless enum signatures now have a real nominal i32 representation;
    // they are never implicitly rewritten as plain i64.
    let src = "define enum Status as Ready end\nfnc show(Status status): int { return 0; }\npub fnc main(): int { return 42; }";
    assert!(atto_compiler::compile(src).is_ok());
}

#[test]
fn definition_quotas_and_error_recovery() {
    let many = (0..257).map(|i| format!("V{i}")).collect::<Vec<_>>().join("\n");
    let src = format!("define enum TooMany as\n{many}\nend\npub fnc main(): int {{ return 1; }}");
    assert_eq!(atto_compiler::compile(&src).unwrap_err().code, "E_SOURCE_LIMIT");
    assert!(atto_compiler::compile(DECLS).is_ok(), "compiler recovers after an error");
    let declarations = (0..1025).map(|i| format!("define class C{i} as end\n")).collect::<String>();
    let src = format!("{declarations}pub fnc main(): int {{ return 1; }}");
    assert_eq!(atto_compiler::compile(&src).unwrap_err().code, "E_SOURCE_LIMIT");
}
