//! Compile-test the bounded immutable-class subset for native and wasm32 hosts.
use atto_compiler::{ast::{DeclarationKind, Type}, lexer, parser::Parser};

const POINT: &str = r#"
define class Point as
  int x: 40
  int y: 2
  bool active: true
end

fnc total(Point point): int {
  return point.x + point.y;
}
pub fnc main(): int {
  val point: Point = Point();
  if point.active then return total(point); else return 0; fi
}
"#;

#[test]
fn nominal_class_fields_have_a_real_typed_layout() {
    let program = Parser::new(lexer::lex(POINT).unwrap()).parse_program().unwrap();
    let class = &program.declarations[0];
    assert_eq!(class.kind, DeclarationKind::Class);
    assert_eq!(class.fields.len(), 3);
    assert_eq!(class.fields[0].name, "x");
    assert_eq!(class.fields[1].name, "y");
    assert_eq!(class.fields[2].ty, Type::Bool);
    assert_eq!(program.funcs[0].params[0].ty, Type::Class(0));
    let wasm = atto_compiler::compile(POINT).expect("compile to Wasm");
    assert_eq!(&wasm[..8], b"\0asm\x01\0\0\0");
    assert_eq!(wasm, atto_compiler::compile(POINT).unwrap());
}

#[test]
fn illegal_class_programs_fail_before_emitting_wasm() {
    const CLASS: &str = "define class P as\n int x: 41\n bool live: true\nend\n";
    const CASES: &[(&str, &str)] = &[
        ("val p = P(); return p.missing;", "E_UNKNOWN_FIELD"),
        ("val p = P(1); return 0;", "E_ARITY"),
        ("val p = P(); return p;", "E_TYPE"),
        ("val p: int = P(); return p;", "E_TYPE"),
        ("val p = P(); if p eq P() then return 1; fi return 0;", "E_UNSUPPORTED"),
        ("val p = P(); return p.x + p.live;", "E_TYPE"),
        ("val P = 3; return P;", "E_DUPLICATE"),
        ("stdio.println(P()); return 0;", "E_UNSUPPORTED"),
        ("return P;", "E_TYPE"),
    ];
    for (body, code) in CASES {
        let src = format!("{CLASS}pub fnc main(): int {{ {body} }}");
        let d = atto_compiler::compile(&src).expect_err(body);
        assert_eq!(d.code, *code, "{body}: {d}");
        assert!(d.pos.line >= 1 && d.pos.col >= 1);
    }
}

#[test]
fn class_layout_and_semantics_reject_unsafe_fields() {
    let cases = [
        ("define class P as\n int x: 0\n int x: 1\nend\n", "E_DUPLICATE"),
        ("define class P as\n let int x: 0\nend\n", "E_UNSUPPORTED"),
        ("define class P as\n int x: 3 + 2\nend\n", "E_UNSUPPORTED"),
        ("define class P as\n int x: true\nend\n", "E_TYPE"),
        ("define class P as\n bool live: 1\nend\n", "E_TYPE"),
        ("define class P as\n String name: \"hello\"\nend\n", "E_UNSUPPORTED"),
        ("define class P as\n fnc method(): int { return 3; }\nend\n", "E_UNSUPPORTED"),
        ("define class P as\n int x: 1 int y: 2\nend\n", "E_SYNTAX"),
        ("define class P as\n int x: 1\n", "E_EOF"),
    ];
    for (prefix, code) in cases {
        let src = format!("{prefix}pub fnc main(): int {{ return 0; }}");
        let d = atto_compiler::compile(&src).expect_err(prefix);
        assert_eq!(d.code, code, "{prefix}: {d}");
    }
    assert!(atto_compiler::compile(POINT).is_ok(), "compiler remains usable after errors");
}
