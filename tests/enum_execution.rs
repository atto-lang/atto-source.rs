//! Compiler-backed enum regression: actual typed IR and binary WebAssembly.
use atto_compiler::{ast::{ExprKind, Type}, lexer, parser::Parser};

const SOURCE: &str = r#"
fnc accepted(Status status): bool { return status eq Status.Ready; }
pub fnc main(): int {
  val selected: Status = Status.Ready;
  if accepted(selected) then return 42; else return 0; fi
}
define enum Status as
  Pending
  Ready
  Failed
end
"#;

#[test]
fn enum_types_are_nominal_and_forward_declarations_resolve() {
    let ast = Parser::new(lexer::lex(SOURCE).unwrap()).parse_program().unwrap();
    assert_eq!(ast.funcs[0].params[0].ty, Type::Enum(0));
    assert_eq!(ast.declarations[0].variants, ["Pending", "Ready", "Failed"]);
    let bytes = atto_compiler::compile(SOURCE).unwrap();
    assert_eq!(&bytes[..8], b"\0asm\x01\0\0\0");
    assert_eq!(bytes, atto_compiler::compile(SOURCE).unwrap(), "deterministic binary emission");
}

#[test]
fn enums_reject_cross_type_mixups_and_invalid_values_without_wasm() {
    const TYPES: &str = "define enum Status as Ready, Failed end\ndefine enum Color as Red, Green end\n";
    const CASES: &[(&str, &str)] = &[
        ("val x: Status = Color.Red; return 0;", "E_TYPE"),
        ("if Status.Ready eq Color.Red then return 1; fi return 0;", "E_TYPE"),
        ("val x: int = Status.Ready; return x;", "E_TYPE"),
        ("return Status.Ready;", "E_TYPE"),
        ("return Status.Unknown;", "E_UNKNOWN_VARIANT"),
        ("stdio.println(Status.Ready); return 0;", "E_UNSUPPORTED"),
        ("val Status = 7; return Status;", "E_DUPLICATE"),
        ("return Status;", "E_TYPE"),
        ("return Status.Ready + 1;", "E_TYPE"),
        ("return Status.Ready < Status.Failed;", "E_TYPE"),
        ("return Color.Blue;", "E_UNKNOWN_VARIANT"),
    ];
    for (body, expected) in CASES {
        let source = format!("{TYPES}pub fnc main(): int {{ {body} }}");
        let err = atto_compiler::compile(&source).expect_err(body);
        assert_eq!(err.code, expected, "{body}: {err}");
        assert!(err.pos.line > 0 && err.pos.col > 0);
    }
}

#[test]
fn enum_variant_constructor_and_member_surface_is_fail_closed() {
    let cases = [
        "define enum E as A end\npub fnc main(): int { val x = E.A(3); return 0; }",
        "define enum E as A end\npub fnc main(): int { val x = E.A.name; return 0; }",
        "define class C as\n int x: 0\nend\npub fnc main(): int { return 0; }",
        "define actor A as\n receive(int x) -> void :: return; end\nend\npub fnc main(): int { return 0; }",
    ];
    for source in cases {
        assert_eq!(atto_compiler::compile(source).unwrap_err().code, "E_UNSUPPORTED", "{source}");
    }
    assert!(atto_compiler::compile(SOURCE).is_ok(), "compiler recovers after rejects");
}
