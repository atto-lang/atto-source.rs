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
    for s in [
        r#"
            define struct First as int x; int y; end
            define struct Second as int x; int y; end
            pub fnc main(): int {
              val v = infer struct{x:2,y:3};
              return 0;
            }
        "#,
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
