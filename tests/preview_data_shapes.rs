//! Bounded source-local product shapes execute through the ordinary checked
//! class constructor, field-read, object-print and real guest Wasm paths.
use atto_compiler::compile;
const SITE: &str = include_str!("playground_previews/data-shapes.atto");
fn pass(s: &str) {
    let bytes=compile(s).unwrap_or_else(|e|panic!("expected compiled guest: {e}"));
    assert!(bytes.starts_with(b"\0asm\x01\0\0\0"));
    assert_eq!(bytes,compile(s).unwrap(),"Wasm guest must be deterministic");
}
fn reject(s: &str) {
    let d=compile(s).expect_err("unsafe or unsupported product shape must emit no guest");
    assert!(d.code.starts_with("E_"),"{d}");
    assert!(d.pos.line>0 && d.pos.col>0,"{d}");
}
fn reject_code(s: &str, code: &str) {
    let d=compile(s).expect_err("invalid product must fail without guest artifact");
    assert_eq!(d.code, code, "unexpected diagnostic: {d}");
}

#[test]
fn exact_published_product_shapes_compile_to_actual_guest_wasm() {
    pass(SITE);
}

#[test]
fn scalar_struct_and_record_constructors_are_typechecked() {
    pass(r#"
        define struct Point as int x; int y; end
        define record Receipt as int id; string status; end
        pub fnc main(): int {
            val point = infer struct{ y: 2, x: 40 };
            val receipt = Receipt{status:"ok",id:7};
            stdio.println(receipt.status);
            return point.x + point.y;
        }
    "#);
}

#[test]
fn positional_products_are_nominal_two_field_guest_objects() {
    pass(r#"
        pub fnc main(): int {
            val a: Tuple<int, string> = (42, "ok");
            val b: Pair<string, int> = Pair("answer", 42);
            val c: Entry<string, int> = Entry("answer", 42);
            stdio.println(a);
            stdio.println(b);
            stdio.println(c);
            return 0;
        }
    "#);
}

#[test]
fn structurally_ambiguous_or_incomplete_keyed_products_reject() {
    // Same-shape nominal declarations do not make an anonymous literal
    // ambiguous; the literal has its own private per-site identity.
    pass(r#"
        define struct First as int x; int y; end
        define struct Second as int x; int y; end
        pub fnc main(): int {
            val v = infer struct{x:2,y:3};
            return v.x + v.y;
        }
    "#);
    for s in [
        r#"
            define struct Point as int x; int y; end
            pub fnc main(): int { val v=Point{x:2}; return 0; }
        "#,
        r#"
            define record Receipt as int id; string status; end
            pub fnc main(): int {
              val v=Receipt{id:2,id:3,status:"x"};
              return 0;
            }
        "#,
        r#"
            define struct Point as int x; int x; end
            pub fnc main(): int { return 0; }
        "#,
        r#"
            define record Receipt as int id; string status; end
            pub fnc main(): int {
              val v=Receipt{id:2,status:42};
              return 0;
            }
        "#,
    ] {reject(s);}
}

#[test]
fn dynamic_and_mismatched_generic_products_fail_without_artifacts() {
    for s in [
        "pub fnc main(): int { val t: Tuple<int,string> = (42); return 0; }",
        "pub fnc main(): int { val t: Tuple<int,string> = (42,42); return 0; }",
        "pub fnc main(): int { val p: Pair<string,int> = Entry(\"x\",42); return 0; }",
        "pub fnc main(): int { val p: Pair<int> = Pair(42); return 0; }",
        "pub fnc main(): int { val p: Pair<Any,int> = Pair(1,42); return 0; }",
        r#"
           define struct Point as int x; int y; end
           pub fnc main(): int {
              val x=40;
              val p=infer struct{x:x,y:2};
              return p.x;
           }
        "#,
        r#"
           define struct Point as int x; int y; end
           pub fnc main(): int {
              val p=infer struct{x:40,y:2};
              p.x=99;
              return p.x;
           }
        "#,
    ] {reject(s);}
}

#[test]
fn forged_internal_product_classes_and_unadmitted_source_fail() {
    reject(r#"
       define class __atto_shape_pair_string_int as end
       pub fnc main(): int { return 0; }
    "#);
    reject("pub fnc main(): int { val x=__atto_shape_pair_string_int; return 0; }");
    reject("pub fnc main(): int { val t=Pair(42,10); return 0; }");
    pass("pub fnc main(): int { return 42; }");
    pass(include_str!("playground_previews/match-types-is.atto"));
}

#[test]
fn explicit_expected_struct_uses_nominal_type_without_shape_guessing() {
    pass(r#"
        define struct First as int x; int y; end
        define struct Second as int x; int y; end
        pub fnc main(): int {
            const First first = infer struct { y: 2, x: 40 };
            val second: Second = infer struct { x: 4, y: 3 };
            return first.x + second.y;
        }
    "#);
}

#[test]
fn anonymous_structs_synthesize_literal_field_types_without_named_declarations() {
    pass(r#"
        pub fnc main(): int {
            const value = infer struct { x: 42, name: "Lima", active: true };
            stdio.println(value.name);
            return value.x;
        }
    "#);
    pass(r#"
        pub fnc main(): int {
            val one = infer struct { x: 1 };
            val two = infer struct { x: 41 };
            return one.x + two.x;
        }
    "#);
}

#[test]
fn empty_anonymous_and_empty_named_structs_are_real_products() {
    pass(r#"
        define struct Empty as end
        pub fnc main(): int {
            const Empty named = infer struct {};
            const anon = infer struct {};
            return 5;
        }
    "#);
}

#[test]
fn struct_inference_rejects_incorrect_fields_and_unsafe_nominal_coercion() {
    for src in [
        r#"
            define struct Point as int x; int y; end
            pub fnc main(): int {
                const Point p = infer struct { x: 5 };
                return p.x;
            }
        "#,
        r#"
            define struct Point as int x; end
            pub fnc main(): int {
                const Point p = infer struct { x: 5, unknown: 9 };
                return p.x;
            }
        "#,
        r#"
            pub fnc main(): int {
                const s = infer struct { x: 1, x: 2 };
                return 0;
            }
        "#,
        r#"
            define struct Point as int x; int y; end
            pub fnc take_point(Point p): int { return p.x; }
            pub fnc main(): int {
                const p = infer struct { x: 1, y: 2 };
                return take_point(p);
            }
        "#,
        r#"
            pub fnc main(): int {
                const s = infer struct { "quoted-key": 9 };
                return 5;
            }
        "#,
        r#"
            pub fnc main(): int {
                const s = infer struct { x: None };
                return 5;
            }
        "#,
    ] {
        reject(src);
    }
}

#[test]
fn unsupported_runtime_computed_struct_fields_stay_fail_closed() {
    reject(r#"
        pub fnc main(): int {
            val x = 40;
            val p = infer struct { x: x, y: 2 };
            return p.y;
        }
    "#);
}

#[test]
fn inferred_struct_errors_are_specific_and_deterministic() {
    reject_code(r#"
        define struct Point as int x; int y; end
        pub fnc main(): int {
            const Point p = infer struct { x: 2 };
            return 0;
        }
    "#, "E_STRUCT_MISSING_FIELD");
    reject_code(r#"
        define struct Point as int x; end
        pub fnc main(): int {
            const Point p = infer struct { x: 2, unknown: 3 };
            return 0;
        }
    "#, "E_NO_FIELD");
    reject_code(r#"
        pub fnc main(): int {
            const anon = infer struct { missing: None };
            return 0;
        }
    "#, "E_TYPE_INFER_AMBIGUOUS");
    reject_code(r#"
        pub fnc main(): int {
            const anon = infer struct { "unusual name": 3 };
            return 0;
        }
    "#, "E_STRUCT_FIELD_NAME");
}

#[test]
fn same_shape_anonymous_values_remain_distinct_nominal_types() {
    reject(r#"
        pub fnc main(): int {
            let first = infer struct { x: 1 };
            val second = infer struct { x: 2 };
            first = second;
            return 0;
        }
    "#);
}
