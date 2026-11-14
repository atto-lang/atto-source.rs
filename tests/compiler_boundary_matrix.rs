//! Compiler-backed Rust regressions for the currently admitted scalar subset.
//! These tests use the real parser, MIR admission, effect exporter and Cranelift
//! object emitter. They do NOT exercise the full ores.std/testing guest module.
use oreslang_source_rs::{compile, compile_with_effect_ir, effects, frontend, mir};

fn checked(source: &str) -> Result<mir::Module, String> {
    let program = frontend::parse(source).map_err(|error| error.to_string())?;
    mir::lower(program).map_err(|error| error.to_string())
}

fn assert_diagnostic(source: &str, class: &str) {
    let error = compile(source).expect_err("invalid program compiled successfully");
    assert!(
        error.starts_with(class),
        "expected diagnostic {class:?}, observed {error:?} for {source:?}"
    );
}

#[test]
fn object_emission_accepts_forward_and_nested_calls() {
    for source in [
        "pub fnc main(): int { return twice(21); } fnc twice(int x): int { return x+x; }",
        "pub fnc main(): int { return sum(20,22); } fnc sum(int a,int b): int { return a+b; }",
        "pub fnc main(): int { return plus_one(plus_one(40)); } fnc plus_one(int a): int { return a+1; }",
        "pub fnc main(): int { return ten()*4+2; } fnc ten(): int { return 10; }",
    ] {
        let obj = compile(source).unwrap_or_else(|error| panic!("{source:?}: {error}"));
        assert!(!obj.is_empty(), "no native object emitted for {source:?}");
    }
}

#[test]
fn object_emission_accepts_exact_i64_limits_and_integer_only_syntax() {
    for source in [
        "pub fnc main(): int { return -9223372036854775808; }",
        "pub fnc main(): int { return 9223372036854775807; }",
        "pub fnc main(): int { return 9_007_199_254_740_993-9_007_199_254_740_992; }",
        "pub fnc main(): int { return -(7*6)+84; }",
        "pub fnc main(): int { const x: int = 40; const y: int = x+2; return y; }",
    ] {
        let obj = compile(source).unwrap_or_else(|error| panic!("{source:?}: {error}"));
        assert!(!obj.is_empty());
    }
}

#[test]
fn parser_preserves_i64_minimum_without_float_rounding() {
    let program = frontend::parse("fnc edge(): int { return -9223372036854775808; }").unwrap();
    assert_eq!(program.functions[0].expression, frontend::Expr::Integer(i64::MIN));
}

#[test]
fn parser_preserves_i64_above_double_mantissa() {
    let program = frontend::parse("fnc edge(): int { return 9_007_199_254_740_993; }").unwrap();
    assert_eq!(
        program.functions[0].expression,
        frontend::Expr::Integer(9_007_199_254_740_993)
    );
}

#[test]
fn nested_comments_cannot_inject_tokens_or_braces() {
    let source = "/* fnc evil(): int { return 1; } /* nested */ } */ pub fnc main(): int { return 42; }";
    let program = frontend::parse(source).unwrap();
    assert_eq!(program.functions.len(), 1);
    assert_eq!(program.functions[0].name, "main");
    assert!(!compile(source).unwrap().is_empty());
}

#[test]
fn malformed_or_unsupported_grammar_is_not_accepted() {
    for source in [
        "define actor A as end",
        "fnc f(): int { return 1/0; }",
        "fnc f(): int { return 1%2; }",
        "fnc f(): int { return 1^2; }",
        "fnc f(): int { return 1^^2; }",
        "fnc f(): int { return 1.5; }",
        "fnc f(): int { return true; }",
        "fnc f(): int { return 4; ",
        "fnc f(): int { return 1; } trailing",
        "",
    ] {
        assert!(compile(source).is_err(), "unexpected admission: {source:?}");
    }
}

#[test]
fn name_and_arity_diagnostics_are_fail_closed() {
    for source in [
        "fnc f(): int { return absent; }",
        "fnc f(): int { return absent(1); }",
        "fnc f(): int { const a: int = b; return a; }",
    ] {
        assert_diagnostic(source, "unknown_name");
    }
    for source in [
        "fnc id(int x): int { return x; } fnc f(): int { return id(); }",
        "fnc id(int x): int { return x; } fnc f(): int { return id(1,2); }",
        "fnc f(): int { return f(1); }",
    ] {
        assert_diagnostic(source, "call_arity");
    }
}

#[test]
fn duplicate_function_parameter_and_local_are_rejected() {
    for source in [
        "fnc f(): int { return 1; } fnc f(): int { return 2; }",
        "fnc f(int a,int a): int { return a; }",
        "fnc f(int x): int { const x: int = 2; return x; }",
        "fnc f(): int { const x: int = 1; const x: int = 2; return x; }",
    ] {
        assert_diagnostic(source, "duplicate_declaration");
    }
}

#[test]
fn integer_literal_overflow_reports_specific_class() {
    for source in [
        "fnc f(): int { return 9223372036854775808; }",
        "fnc f(): int { return -9223372036854775809; }",
        "fnc f(): int { return 99_999_999_999_999_999_999; }",
    ] {
        assert_diagnostic(source, "integer_overflow");
    }
}

#[test]
fn unterminated_nested_comment_is_not_silently_ignored() {
    assert_diagnostic(
        "fnc f(): int { return 7; } /* outer /* inner */",
        "unterminated_comment",
    );
}

#[test]
fn resource_limits_reject_before_object_emission() {
    let excessive_source = " ".repeat(frontend::MAX_SOURCE_BYTES + 1);
    assert_diagnostic(&excessive_source, "resource_limit");

    let deep = format!(
        "fnc f(): int {{ return {}1{}; }}",
        "(".repeat(frontend::MAX_PAREN_NESTING + 1),
        ")".repeat(frontend::MAX_PAREN_NESTING + 1)
    );
    assert_diagnostic(&deep, "resource_limit");

    let many_tokens = format!(
        "fnc f(): int {{ return {}1; }}",
        "1+".repeat(frontend::MAX_TOKENS)
    );
    assert_diagnostic(&many_tokens, "resource_limit");
}

#[test]
fn accepted_source_does_not_skip_mir_or_effect_admission() {
    let source = "pub fnc main(): int { return add(40,2); } fnc add(int a,int b): int { return a+b; }";
    let mir = checked(source).unwrap();
    let independently_exported = effects::export_effect_ir_checked(&mir).unwrap();
    let (object, paired_effects) = compile_with_effect_ir(source).unwrap();
    assert!(!object.is_empty());
    assert_eq!(paired_effects, independently_exported);
    assert_eq!(paired_effects["functions"][0]["operations"][2]["kind"], "call");
    assert_eq!(paired_effects["functions"][0]["operations"][2]["target"], "add");
}

#[test]
fn local_initializers_are_accounted_for_in_effect_ir() {
    let source = "pub fnc main(): int { const x: int = 10+2; return x*3; }";
    let (_object, ir) = compile_with_effect_ir(source).unwrap();
    let operations = ir["functions"][0]["operations"].as_array().unwrap();
    // Two constants and add, then local lookup, constant and multiply.
    assert_eq!(operations.len(), 6);
    assert!(operations.iter().all(|op| op["kind"] == "compute"));
}

#[test]
fn effect_budget_rejects_adversarial_mir_without_recursive_export() {
    let mut expr = frontend::Expr::Integer(1);
    for _ in 0..80 {
        expr = frontend::Expr::Negate(Box::new(expr));
    }
    let module = mir::Module {
        exports: vec![],
        functions: vec![mir::Function {
            symbol: "edge".to_string(),
            exported: true,
            params: vec![],
            locals: vec![],
            return_value: expr,
        }],
        managed_refs: Default::default(),
    };
    let error = effects::export_effect_ir_checked(&module).unwrap_err();
    assert!(error.contains("depth") || error.contains("budget"), "{error}");
}

#[test]
fn empty_function_program_does_not_produce_native_object() {
    assert_diagnostic(" /* comments only */ ", "syntax_error");
}
