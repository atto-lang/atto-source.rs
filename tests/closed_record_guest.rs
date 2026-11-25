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
fn unicode_identifier_record_fields_have_checked_guest_loads() {
    wasm(
        r#"
        pub fnc main(): int {
          let Record headers = {
            content_type: "application/json",
            über_field: 40,
            status_code: 2
          };
          stdio.println(headers["content_type"]);
          return headers["über_field"] + headers.status_code;
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
fn known_scalar_record_type_slots_compile() {
    wasm(
        r#"
        pub fnc main(): int {
          val Record r = {empty: 40, enabled: true, ratio: 1.5};
          return r["empty"];
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
        "E_LITERAL_KIND",
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
        "E_IMMUTABLE",
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
    let source = "pub fnc main(): int {let Record r = {x_y: 42}; return r[\"x_y\"];}";
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
            assert!(
                matches!(&v.kind, ExprKind::Index(_, _)),
                "compiler lowering must preserve the source Index until typed checking"
            )
        }
        _ => panic!("record bracket syntax must survive lowering unchanged"),
    }
    wasm(source);
}

#[test]
fn record_dot_and_bracket_static_fields_both_compile_as_executable_expressions() {
    wasm(
        r#"
        pub fnc main(): int {
            let Record headers = {
                status_code: 42,
                content_type: "application/json"
            };
            headers.status_code;
            headers["content_type"];
            return headers.status_code;
        }
    "#,
    );
}

#[test]
fn bracket_access_stays_typed_in_nested_lexical_blocks() {
    wasm(
        r#"
        pub fnc main(): int {
            let Record headers = {
                content_type: "application/json",
                status_code: 42
            };
            block {
                stdio.println(headers["content_type"]);
                headers.status_code;
            }
            return headers["status_code"];
        }
    "#,
    );
}

#[test]
fn local_alias_of_a_closed_immutable_record_keeps_static_field_access() {
    wasm(
        r#"
        pub fnc main(): int {
            let Record headers = {content_type: 42, status_code: 42};
            val alias = headers;
            return alias["content_type"];
        }
    "#,
    );
}

#[test]
fn equivalent_dot_and_quoted_identifier_field_names_have_same_semantics() {
    wasm(
        r#"
        pub fnc main(): int {
            let Record headers = {status_code: 42, content_type: "ok"};
            return headers.status_code + headers["status_code"] - 42;
        }
    "#,
    );
}

#[test]
fn shadowed_record_name_cannot_grant_bracket_access_to_a_scalar() {
    invalid(
        r#"
        pub fnc main(): int {
            let Record headers = {content_type: 42};
            block {
                let headers = 7;
                stdio.println(headers["content_type"]);
            }
            return 0;
        }
    "#,
        "E_UNSUPPORTED",
    );
}

#[test]
fn nominal_struct_and_normal_class_are_not_reinterpreted_as_record_indexing() {
    invalid(
        r#"
        define struct Header as int status_code; end
        pub fnc main(): int {
            val header = Header{status_code: 42};
            return header["status_code"];
        }
    "#,
        "E_UNSUPPORTED",
    );
}

#[test]
fn string_expression_keys_remain_dynamic_and_unsupported() {
    invalid(
        r#"
        pub fnc main(): int {
            let Record headers = {content_type: 42};
            return headers["content" + "-type"];
        }
    "#,
        "E_UNSUPPORTED",
    );
}

#[test]
fn unicode_record_identifier_fields_remain_distinct() {
    wasm(
        r#"
        pub fnc main(): int {
            let Record headers = {
                empty: 1,
                café: 41,
                cafe_decomp: 7
            };
            return headers["empty"] + headers["café"];
        }
    "#,
    );
    invalid(
        r#"
        pub fnc main(): int {
            let Record headers = {café: 42};
            return headers["cafe_decomp"];
        }
    "#,
        "E_NO_FIELD",
    );
}

#[test]
fn dot_and_static_bracket_share_exact_typed_field_offset_and_type() {
    use atto_compiler::ast::Type;
    use atto_compiler::check::{self, TExpr, TStmt};

    let source = r#"
        pub fnc main(): int {
            let Record headers = {
                content_type: "application/json",
                status_code: 200
            };
            headers.status_code;
            headers["status_code"];
            headers["content_type"];
            return 0;
        }
    "#;
    let parsed = Parser::new(lexer::lex(source).unwrap())
        .parse_program()
        .unwrap();
    let collected = collection_types::analyze(&parsed).unwrap();
    let mut lowered = parsed;
    closed_record_guest::lower(&mut lowered, &collected).unwrap();
    let typed = check::check(&lowered).unwrap();
    let main = typed.funcs.iter().find(|f| f.name == "main").unwrap();
    let reads: Vec<(u32, Type)> = main
        .body
        .iter()
        .filter_map(|stmt| match stmt {
            TStmt::Expr(v) => match &v.e {
                TExpr::FieldGet { offset, .. } => Some((*offset, v.ty)),
                _ => None,
            },
            _ => None,
        })
        .collect();
    assert_eq!(
        reads.len(),
        3,
        "both notations should use typed direct-offset reads"
    );
    assert_eq!(
        reads[0], reads[1],
        "dot and quoted field name have one type and offset"
    );
    assert_eq!(reads[0].1, Type::Int);
    assert_eq!(reads[2].1, Type::Str);
    assert_ne!(
        reads[0].0, reads[2].0,
        "distinct fields cannot alias a slot"
    );
}

#[test]
fn identifier_record_field_names_share_static_bracket_alias() {
    wasm(
        r#"
        pub fnc main(): int {
            let Record headers = {
                x_y: 41,
                has_quote: 1,
                line_break: 2
            };
            return headers["x_y"] + headers["has_quote"];
        }
    "#,
    );
    invalid(
        r#"
        pub fnc main(): int {
            let Record headers = {x_y: 1, x_y: 2};
            return 0;
        }
    "#,
        "E_DUPLICATE",
    );
    invalid(
        r#"
        pub fnc main(): int {
            let Record headers = {x_y: 42};
            return headers["x\u{zz}y"];
        }
    "#,
        "E_STRING",
    );
}

#[test]
fn static_bracket_read_infers_field_type_in_return_checker() {
    invalid(
        r#"
        pub fnc main(): int {
            let Record headers = {
                content_type: "application/json",
                status_code: 200
            };
            return headers["content_type"];
        }
    "#,
        "E_TYPE",
    );
    wasm(
        r#"
        pub fnc main(): int {
            let Record headers = {content_type: "application/json", status_code: 42};
            return headers["status_code"];
        }
    "#,
    );
}


#[test]
fn checked_owned_scalar_record_writes_emit_guest_wasm() {
    for source in [
        r#"pub fnc main(): int {
            let mut Record r = {foo: 1};
            r["foo"] = 42;
            return r.foo;
        }"#,
        r#"pub fnc main(): int {
            val mut Record r = {content_type: 1};
            r["content_type"] = 42;
            return r["content_type"];
        }"#,
        r#"pub fnc main(): int {
            let mut Record r = {foo: 1, bar: 9};
            r.foo = 33;
            return r["foo"] + r.bar;
        }"#,
        r#"pub fnc main(): int {
            let mut Record r = {empty: 1, é: 2, e_decomp: 3};
            r["empty"] = 37;
            r["e_decomp"] = 3;
            return r["empty"] + r["é"] + r["e_decomp"];
        }"#,
    ] {
        wasm(source);
    }
}

#[test]
fn record_write_fail_closed_immutable_type_missing_or_computed() {
    for (source, code) in [
        (r#"pub fnc main(): int { let Record r={foo:1}; r["foo"]=42; return 0; }"#, "E_IMMUTABLE"),
        (r#"pub fnc main(): int { val Record r={foo:1}; r.foo=42; return 0; }"#, "E_IMMUTABLE"),
        (r#"pub fnc main(): int { let mut Record r={foo:1}; r["bar"]=42; return 0; }"#, "E_NO_FIELD"),
        (r#"pub fnc main(): int { let mut Record r={foo:1}; r["foo"]="bad"; return 0; }"#, "E_TYPE"),
        (r#"pub fnc main(): int { let mut Record r={foo:1}; let string key="foo"; r[key]=42; return 0; }"#, "E_UNSUPPORTED"),
        (r#"pub fnc main(): int { let mut Record r={foo:1}; r[0]=42; return 0; }"#, "E_UNSUPPORTED"),
        (r#"pub fnc main(): int { let mut Record r={foo:1}; r["f"+"oo"]=42; return 0; }"#, "E_UNSUPPORTED"),
        (r#"pub fnc main(): int { let mut Record r={foo:1}; val moved=take! r; r["foo"]=42; return 0; }"#, "E_MOVED"),
        (r#"pub fnc main(): int { let mut Record r={foo:1}; val reader=borrow! r; r["foo"]=42; return 0; }"#, "E_BORROW"),
        (r#"define struct Point as int foo; end pub fnc main(): int { let mut p=Point{foo:1}; p["foo"]=42; return 0; }"#, "E_UNSUPPORTED"),
        (r#"define struct Point as int foo; end pub fnc main(): int { val p=Point{foo:1}; return p["foo"]; }"#, "E_UNSUPPORTED"),
    ] {
        invalid(source,code);
    }
}

#[test]
fn index_write_retains_field_type_and_lexical_class_identity() {
    let p = Parser::new(lexer::lex(
        r#"pub fnc main(): int {
            let mut Record r={foo:1,a_b:9};
            r["a_b"]=42;
            return r["a_b"];
        }"#).unwrap()).parse_program().unwrap();
    let checked = collection_types::analyze(&p).unwrap();
    let mut lowered=p.clone();
    closed_record_guest::lower(&mut lowered, &checked).unwrap();
    assert!(matches!(&lowered.funcs[0].body.last().unwrap(),
        Stmt::Return(Some(v),_) if matches!(&v.kind,ExprKind::Index(_, _))));
    assert!(lowered.classes.iter().any(|c| c.name.starts_with(
        closed_record_guest::PRIVATE_PREFIX)));
}

#[test]
fn nested_field_index_and_computed_map_key_stay_fail_closed() {
    invalid(
        r#"
        pub fnc main(): int {
            let Record headers = {content_type: 42};
            return headers["content_type"][0];
        }
    "#,
        "E_UNSUPPORTED",
    );
    invalid(
        r#"
        pub fnc main(): int {
            let Record headers = {content_type: 42};
            let string field = "content_type";
            return headers[field];
        }
    "#,
        "E_UNSUPPORTED",
    );
}
