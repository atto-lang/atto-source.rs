//! Real compiler/guest admission for a narrow, compiler-authenticated closed
//! Record scalar profile. These tests do NOT imply open Records, heap graphs,
//! actor transport, mutable fields or general collection support.

use atto_compiler::closed_record_guest;
use atto_compiler::{
    ast::{ExprKind, Stmt},
    collection_types, compile, lexer,
    parser::Parser,
};

fn wasm(source: &str) -> Vec<u8> {
    let actual = compile(source).unwrap_or_else(|err| panic!("{source}\n{err}"));
    assert!(actual.starts_with(b"\0asm\x01\0\0\0"));
    assert_eq!(
        actual,
        compile(source).unwrap(),
        "guest compiler nondeterminism"
    );
    actual
}
fn invalid(source: &str, expected: &str) {
    let err = compile(source).expect_err("unsupported/unsafe Record must emit no guest");
    assert_eq!(err.code, expected, "{source}\n{err}");
}

#[test]
fn real_guest_compiles_canonical_closed_record_dot_fields() {
    wasm(
        r#"
        pub fnc main(): int {
          let Record point = { y: 2, x: 40 };
          return point.x + point.y;
        }
    "#,
    );
}

#[test]
fn quoted_utf8_names_have_checked_direct_guest_field_loads() {
    wasm(
        r#"
        pub fnc main(): int {
          let Record headers = {
            "content-type": "application/json",
            "über field": 40,
            status_code: 2
          };
          stdio.println(headers["content-type"]);
          return headers["über field"] + headers.status_code;
        }
    "#,
    );
}

#[test]
fn two_source_permutations_share_one_generated_record_shape() {
    let source = r#"
        pub fnc main(): int {
          let Record first = { z: true, a: 40 };
          let record second = { a: 41, z: false };
          return first.a + second.a - 39;
        }
    "#;
    let program = Parser::new(lexer::lex(source).unwrap())
        .parse_program()
        .unwrap();
    let typed = collection_types::analyze(&program).unwrap();
    let mut lowered = program.clone();
    closed_record_guest::lower(&mut lowered, &typed).unwrap();
    assert_eq!(lowered.classes.len(), program.classes.len() + 1);
    assert_eq!(
        lowered
            .classes
            .last()
            .unwrap()
            .fields
            .iter()
            .map(|f| f.name.as_str())
            .collect::<Vec<_>>(),
        vec!["a", "z"]
    );
    assert!(matches!(&lowered.funcs[0].body[2], Stmt::Bind { .. }));
    wasm(source);
}

#[test]
fn known_scalar_type_slots_and_empty_string_label_compile() {
    wasm(
        r#"
        pub fnc main(): int {
          val Record r = {"": 40, enabled: true, ratio: 1.5};
          return r[""];
        }
    "#,
    );
}

#[test]
fn already_declared_nominal_struct_remains_separate_and_executable() {
    wasm(
        r#"
        define struct Point as int x; int y; end
        pub fnc main(): int {
            val p = Point{ y: 2, x: 40 };
            let Record copy = {x: 40, y: 2};
            return p.x + copy.y;
        }
    "#,
    );
}

#[test]
fn maps_open_records_and_computed_indexing_still_fail_closed() {
    invalid(
        r#"
        pub fnc main(): int {
            let Map<string,int> m = {"x" => 42};
            return m["x"];
        }
    "#,
        "E_UNSUPPORTED",
    );
    invalid(
        r#"
        pub fnc main(): int {
            let Record r = {x: 42};
            let string key = "x";
            return r[key];
        }
    "#,
        "E_UNSUPPORTED",
    );
    invalid(
        r#"
        pub fnc main(): int {
            let Record r = {x: 42};
            return r["missing"];
        }
    "#,
        "E_NO_FIELD",
    );
}

#[test]
fn rejected_fields_and_ownership_do_not_emit_guest_wasm() {
    invalid(
        r#"
        pub fnc main(): int {
            let Record r = {x: 2, "x": 3};
            return 0;
        }
    "#,
        "E_DUPLICATE",
    );
    invalid(
        r#"
        pub fnc main(): int {
            let Record r = {a: [1, 2], x: 1};
            return 0;
        }
    "#,
        "E_UNSUPPORTED",
    );
    invalid(
        r#"
        pub fnc main(): int {
            const Record r = {x: 42};
            return r.x;
        }
    "#,
        "E_UNSUPPORTED",
    );
    invalid(
        r#"
        pub fnc main(): int {
            let Record r = {x: 42};
            r.x = 1;
            return r.x;
        }
    "#,
        "E_OWNERSHIP",
    );
}

#[test]
fn mutable_borrow_cannot_turn_closed_record_fields_into_writable_class_storage() {
    invalid(
        r#"
        pub fnc main(): int {
            let Record r = {x: 42};
            let writer = borrow! mut r;
            writer.x = 7;
            return writer.x;
        }
    "#,
        "E_BORROW",
    );
}

#[test]
fn source_cannot_forge_private_generated_class_or_temp_bindings() {
    invalid(
        r#"
        pub fnc main(): int {
            let __atto_closed_record_tmp_0 = 42;
            return __atto_closed_record_tmp_0;
        }
    "#,
        "E_RESERVED",
    );
    invalid(
        r#"
        pub fnc main(): int {
            return new __atto_closed_record_shape_0().x;
        }
    "#,
        "E_RESERVED",
    );
}

#[test]
fn nonliteral_field_effects_are_unproven_and_rejected() {
    invalid(
        r#"
        fnc get_num(): int { return 42; }
        pub fnc main(): int {
            let Record r = {x: get_num()};
            return r.x;
        }
    "#,
        "E_UNSUPPORTED",
    );
}

#[test]
fn record_in_nested_blocks_stays_unsupported_without_lexical_proof() {
    invalid(
        r#"
        pub fnc main(): int {
            block {
                let Record r = {x: 42};
                stdio.println(r.x);
            }
            return 0;
        }
    "#,
        "E_UNSUPPORTED",
    );
}

#[test]
fn dynamic_integer_index_is_not_a_string_field_projection() {
    invalid(
        r#"
        pub fnc main(): int {
            let Record r = {x: 42};
            return r[0];
        }
    "#,
        "E_UNSUPPORTED",
    );
}

#[test]
fn parsed_bracket_index_keeps_provenance_before_private_lowering() {
    let source = "pub fnc main(): int {let Record r = {\"x-y\": 42}; return r[\"x-y\"];}";
    let program = Parser::new(lexer::lex(source).unwrap())
        .parse_program()
        .unwrap();
    match &program.funcs[0].body[1] {
        Stmt::Return(Some(v), _) => assert!(matches!(&v.kind, ExprKind::Index(_, _))),
        _ => panic!("the original parsed Index node must survive until checked lowering"),
    }
    let typed = collection_types::analyze(&program).unwrap();
    let mut copy = program;
    closed_record_guest::lower(&mut copy, &typed).unwrap();
    match &copy.funcs[0].body.last().unwrap() {
        Stmt::Return(Some(v), _) => {
            assert!(matches!(&v.kind, ExprKind::Member(_, field) if field=="x-y"))
        }
        _ => panic!("a proved closed field becomes an ordinary checked member projection"),
    }
}
