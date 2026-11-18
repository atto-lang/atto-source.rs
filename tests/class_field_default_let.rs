//! Class-field default binding mode regression: this is parser/compiler-subset
//! evidence, NOT proof of mutable receiver, val mut, const evaluation or AOT parity.
use atto_compiler::{compile, lexer::lex, parser::Parser};
use atto_compiler::ast::FieldMode;

#[test]
fn bare_class_fields_are_let_for_scalar_string_and_class_types() {
    let source = r#"
        define class Leaf as
          pub int amount = 5;
        end
        define class Holder as
          pub int count = 1;
          pub let int explicit_count = 2;
          pub string name = "example";
          pub Leaf child = new Leaf();
          pub val int stable = 3;
          pub final int fixed = 4;
          pub const int limit = 5;
        end
        pub fnc main(): int { return 0; }
    "#;
    let parsed = Parser::new(lex(source).unwrap()).parse_program().unwrap();
    let holder = parsed.classes.iter().find(|c| c.name == "Holder").unwrap();
    for field_name in ["count", "explicit_count", "name", "child"] {
        let field = holder.fields.iter().find(|f| f.name == field_name).unwrap();
        assert_eq!(field.mode, FieldMode::Constructor,
            "implicit class-field mode must be independent of primitive or object type: {field_name}");
    }
    for field_name in ["stable", "fixed", "limit"] {
        let field = holder.fields.iter().find(|f| f.name == field_name).unwrap();
        assert_eq!(field.mode, FieldMode::Final,
            "existing Rust parser treats val/final/const as immutable: {field_name}");
    }
}

#[test]
fn implicit_and_explicit_let_both_compile_into_guest_wasm() {
    let source = r#"
        define class Counter as
          pub int count = 9;
        end
        pub fnc main(): int {
          val Counter value = new Counter();
          return value.count;
        }
    "#;
    for src in [source.to_owned(), source.replace("pub int count = 9;", "pub let int count = 9;")] {
        let parsed = Parser::new(lex(&src).unwrap()).parse_program().unwrap();
        assert_eq!(parsed.classes[0].fields[0].mode, FieldMode::Constructor);
        let guest = compile(&src).unwrap();
        assert_eq!(&guest[..8], &[0, 97, 115, 109, 1, 0, 0, 0]);
    }
}

#[test]
fn class_val_and_final_fields_cannot_be_reassigned_from_constructor() {
    for modifier in ["val", "final"] {
        let source = format!(r#"
            define class Stable as
              pub {modifier} int id = 1;
              constructor() {{ self.id = 3; }}
            end
            pub fnc main(): int {{ return 0; }}
        "#);
        let diagnostic = compile(&source).unwrap_err();
        assert_eq!(diagnostic.code, "E_IMMUTABLE", "{source}: {diagnostic}");
    }
}

#[test]
fn let_default_does_not_grant_mutable_readonly_receiver() {
    let source = r#"
        define class Counter as
          pub int count = 1;
          change(): int { self.count = 2; return self.count; }
        end
        pub fnc main(): int { return 0; }
    "#;
    let diagnostic = compile(source).unwrap_err();
    assert_eq!(diagnostic.code, "E_OWNERSHIP", "{diagnostic}");
}
