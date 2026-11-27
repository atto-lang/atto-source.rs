//! First executable slice: typed private Wasm direct functions, no captures.
use atto_compiler::{
    ast::Type, check::{self, TExpr, TStmt}, lexer, parser::Parser
};

fn checked(source: &str) -> check::TProgram {
    let p = Parser::new(lexer::lex(source).unwrap()).parse_program().unwrap();
    check::check(&p).unwrap()
}
#[test]
fn direct_lambda_generates_typed_private_function() {
    let p = checked("pub fnc main(): int { val double = |n| : int -> n * 2; return double(21); }");
    assert_eq!(p.funcs.len(), 2);
    assert_eq!(p.funcs[1].name, "$lambda_0");
    assert_eq!(p.funcs[1].params, [Type::Int]);
    assert_eq!(p.funcs[1].ret, Type::Int);
    let TStmt::Return(Some(value)) = &p.funcs[0].body[0] else {
        panic!("expected direct call return")
    };
    assert!(matches!(&value.e, TExpr::Call(1, args) if args.len() == 1));
}
#[test]
fn explicit_statement_return_compiles_to_typed_function() {
    let source = "pub fnc main(): int { val double = (|n| : int -> return n * 2;); return double(21); }";
    let p = checked(source);
    assert_eq!(p.funcs[1].ret, Type::Int);
    assert!(atto_compiler::compile(source).is_ok());
}
#[test]
fn zero_parameter_lambda_and_deterministic_wasm() {
    let src = "pub fnc main(): int { val answer = || -> 42; return answer(); }";
    let wasm = atto_compiler::compile(src).unwrap();
    assert_eq!(atto_compiler::compile(src).unwrap(), wasm);
    assert!(wasm.starts_with(b"\0asm"));
}
#[test]
fn immutable_scalar_capture_is_lifted_as_hidden_wasm_parameter() {
    let src = "pub fnc main(): int { val extra = 1; val f = |n| -> n + extra; return f(1); }";
    let p = checked(src);
    assert_eq!(p.funcs[1].params, [Type::Int, Type::Int]);
    assert!(atto_compiler::compile(src).is_ok());
    let TStmt::Return(Some(value)) = &p.funcs[0].body[2] else {
        panic!("expected direct captured call return")
    };
    assert!(matches!(&value.e, TExpr::Call(1, args) if args.len() == 2));
}
#[test]
fn immutable_function_parameter_and_val_alias_capture_as_definition_snapshot() {
    let src = r#"
      fnc multiply(int factor): int {
        val captured_factor = factor;
        val triple = |value| : int -> value * captured_factor;
        return triple(14);
      }
      pub fnc main(): int { return multiply(3); }
    "#;
    let checked = checked(src);
    // Two nominal functions plus the lifted private direct-call function.
    assert_eq!(checked.funcs.len(), 3);
    assert_eq!(checked.funcs[2].params, [Type::Int, Type::Int]);
    let wasm = atto_compiler::compile(src).unwrap();
    assert_eq!(wasm, atto_compiler::compile(src).unwrap());
    assert!(wasm.starts_with(b"\0asm"));
}
#[test]
fn mutable_source_cannot_be_promoted_through_an_immutable_alias() {
    let src = "pub fnc main(): int { let n = 10; val snapshot = n; val f = |v| -> v + snapshot; return f(1); }";
    assert_eq!(atto_compiler::compile(src).unwrap_err().code, "E_CAPTURE");
}
#[test]
fn mutable_outer_capture_still_rejected() {
    let src = "pub fnc main(): int { let extra = 1; val f = |n| -> n + extra; return f(1); }";
    assert_eq!(atto_compiler::compile(src).unwrap_err().code, "E_CAPTURE");
}

#[test]
fn two_immutable_captures_preserve_lexical_values_under_nested_shadowing() {
    let source = "pub fnc main(): int { val a = 2; val b = 4; val f = |n| : int -> n * a + b; block { val a = 100; block { val b = 200; return f(19); } } }";
    let p = checked(source);
    assert_eq!(p.funcs[1].params, [Type::Int, Type::Int, Type::Int]);
    let TStmt::Block(ref outer) = p.funcs[0].body[4] else {
        panic!("expected lexical shadow block")
    };
    let TStmt::Block(ref inner) = outer[1] else {
        panic!("expected nested lexical shadow block")
    };
    assert!(matches!(&inner[1], TStmt::Return(Some(value))
        if matches!(&value.e, TExpr::Call(1, args) if args.len() == 3)));
    assert!(atto_compiler::compile(source).is_ok());
}

#[test]
fn lowered_lambda_helper_is_not_implicitly_effect_exempt() {
    // The generated $lambda_* is synthetically marked pure, so the fully
    // typed effect checker must still reject its heap-allocating string concat.
    let source = "pub fnc main(): int { val f = |s| : string -> s + \"x\"; stdio.println(f(\"a\")); return 0; }";
    let d = atto_compiler::compile(source).unwrap_err();
    assert_eq!(d.code, "E_PURE", "{d}");
}

#[test]
fn direct_lambda_cannot_read_an_out_of_scope_block_binding() {
    let source = "pub fnc main(): int { block { val extra = 2; } val f = |n| : int -> n + extra; return f(40); }";
    let d = atto_compiler::compile(source).unwrap_err();
    assert_eq!(d.code, "E_CAPTURE", "{d}");
}
#[test]
fn lambda_escape_does_not_silently_create_callable() {
    let src = "pub fnc main(): int { val f = |n| -> n + 1; val alias = f; return f(1); }";
    assert_eq!(atto_compiler::compile(src).unwrap_err().code, "E_UNSUPPORTED");
}
#[test]
fn not_called_lambda_remains_unsupported() {
    let src = "pub fnc main(): int { val f = |n| -> n + 1; return 0; }";
    assert_eq!(atto_compiler::compile(src).unwrap_err().code, "E_UNSUPPORTED");
}


#[test]
fn three_independent_direct_lambdas_lower_into_distinct_wasm_helpers() {
    // No indirect-call table and no JS/host implementation. These are
    // separate compiler-generated typed functions with real Wasm bodies.
    let src = r#"
        pub fnc main(): int {
            val discard = |_| : int -> 5;
            val seven = || -> 7;
            val twice = |n| : int -> n * 2;
            return discard(99) + seven() + twice(15);
        }
    "#;
    let checked = checked(src);
    assert_eq!(checked.funcs.len(), 4);
    assert_eq!(checked.funcs[1].name, "$lambda_0");
    assert_eq!(checked.funcs[2].name, "$lambda_0_1");
    assert_eq!(checked.funcs[3].name, "$lambda_0_2");
    assert_eq!(checked.funcs[1].params, [Type::Int]);
    assert!(checked.funcs[2].params.is_empty());
    assert_eq!(checked.funcs[3].params, [Type::Int]);
    let wasm = atto_compiler::compile(src).expect("real guest module");
    assert_eq!(wasm, atto_compiler::compile(src).unwrap());
    assert!(wasm.starts_with(b"\0asm"));
}

#[test]
fn sibling_lambdas_get_independent_definition_site_capture_snapshots() {
    let src = r#"
        pub fnc main(): int {
            val base = 2;
            val add = |n| : int -> n + base;
            val multiply = |n| : int -> n * base;
            return add(10) + multiply(15);
        }
    "#;
    let p = checked(src);
    assert_eq!(p.funcs.len(), 3);
    assert_eq!(p.funcs[1].params, [Type::Int, Type::Int]);
    assert_eq!(p.funcs[2].params, [Type::Int, Type::Int]);
    let wasm = atto_compiler::compile(src).unwrap();
    assert_eq!(wasm, atto_compiler::compile(src).unwrap());
    assert!(wasm.starts_with(b"\0asm"));
}

#[test]
fn mixed_lambda_expression_statement_and_block_bodies_emit_real_wasm() {
    let source = r#"
        pub fnc main(): int {
            val five = (|n| : int -> return 5;);
            val seven = |n| : int -> { return n + 7; };
            val thirty = |n| : int -> n * 3;
            return five(99) + seven(0) + thirty(10);
        }
    "#;
    let p = checked(source);
    assert_eq!(p.funcs.len(), 4);
    assert_eq!(p.funcs[1].name, "$lambda_0");
    assert_eq!(p.funcs[2].name, "$lambda_0_1");
    assert_eq!(p.funcs[3].name, "$lambda_0_2");
    let wasm = atto_compiler::compile(source).expect("mixed-body source compiler");
    assert_eq!(wasm, atto_compiler::compile(source).unwrap());
    assert!(wasm.starts_with(&[0, 97, 115, 109]));
}

#[test]
fn nested_block_lambda_call_is_rejected_before_wasm_lowering() {
    let source = r#"
        fnc later(int n): int { return n * 100; }
        pub fnc main(): int {
            val first = |n| : int -> {
                if n > 0 then
                    return later(n);
                fi
                return 0;
            };
            val later = |n| : int -> n + 1;
            return first(1) + later(1);
        }
    "#;
    let err = atto_compiler::compile(source).expect_err("nested forward-call bypass");
    assert_eq!(err.code, "E_UNSUPPORTED", "{err}");
}

#[test]
fn statement_body_cannot_bypass_sibling_call_preflight() {
    let src = r#"
        fnc later(int n): int { return n * 100; }
        pub fnc main(): int {
            val first = (|n| : int -> return later(n) + 1;);
            val later = |n| : int -> n + 1;
            return first(1) + later(1);
        }
    "#;
    let d = atto_compiler::compile(src).expect_err("mixed-bodied lambda call graph needs scanning");
    assert_eq!(d.code, "E_UNSUPPORTED", "{d}");
}
#[test]
fn lambda_forward_reference_must_not_resolve_to_an_unrelated_global_function() {
    // The previous source-order scanner only knew already-visited bindings.
    // A future local lambda could accidentally become a global direct call.
    let source = r#"
        fnc later(int n): int { return n * 100; }
        pub fnc main(): int {
            val first = |n| : int -> later(n) + 1;
            val later = |n| : int -> n + 1;
            return first(1) + later(1);
        }
    "#;
    let diag = atto_compiler::compile(source).expect_err("forward callable shadow must reject");
    assert_eq!(diag.code, "E_UNSUPPORTED", "{diag}");
}

#[test]
fn multi_lambda_rejects_unverified_links_shadowing_and_excessive_bindings() {
    for (source, code) in [
        ("pub fnc main(): int { val a = |n| -> n+1; val a = |n| -> n+2; return a(10); }",
            "E_DUPLICATE"),
        ("pub fnc main(): int { val a = |n| -> n+1; val b = |n| -> a(n)+2; return a(2)+b(2); }",
            "E_UNSUPPORTED"),
        ("pub fnc main(): int { val a = |n| -> n+1; val b = |n| -> n+2; return a(41); }",
            "E_UNSUPPORTED"),
    ] {
        let diagnostic = atto_compiler::compile(source).expect_err("must fail closed");
        assert_eq!(diagnostic.code, code, "{diagnostic}");
    }
    let mut bindings = String::new();
    for i in 0..9 {
        use std::fmt::Write as _;
        write!(&mut bindings, "val f_{i} = || -> {i};").unwrap();
    }
    let src = format!("pub fnc main(): int {{ {bindings} return f_0(); }}");
    let diagnostic = atto_compiler::compile(&src).expect_err("per-function helper budget");
    assert_eq!(diagnostic.code, "E_SOURCE_LIMIT", "{diagnostic}");
}

#[test]
fn sibling_typed_call_apply_and_captures_share_ordinary_wasm() {
    let prefix = "pub fnc main():int { val offset = 2; val plus = |n| : int -> n+offset; val double: fnc(int)->int = |n| : int -> n*2; val seven: fnc()->int = || : int -> 7; return ";
    let direct = atto_compiler::compile(&format!("{prefix}plus(3)+double(15)+seven(); }}")).unwrap();
    let intrinsic = atto_compiler::compile(&format!("{prefix}plus.call(3)+double.apply((15,))+seven.call(); }}")).unwrap();
    assert_eq!(direct,intrinsic);
}

#[test]
fn sibling_member_references_are_not_lowered_as_ordinary_globals() {
    for expression in ["later.call(n)", "later.apply((1,))", "later"] {
        let src = format!("fnc later(int n):int {{ return n*100; }} pub fnc main():int {{ val first = |n| : int -> {expression}; val later = |n| : int -> n+1; return first(1)+later(1); }}");
        assert_eq!(atto_compiler::compile(&src).unwrap_err().code,"E_UNSUPPORTED","{src}");
    }
}

#[test]
fn separate_statement_body_is_supported_for_independent_siblings() {
    let src = "pub fnc main(): int { val a = |n| -> n+1; val b = (|n| -> return n+2;); return a(19)+b(20); }";
    assert!(atto_compiler::compile(src).is_ok(),"{src}");
}
