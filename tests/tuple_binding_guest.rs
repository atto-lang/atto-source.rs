//! Executable Rust/native-compiler acceptance and rejection checks.
//! These do not by themselves establish executed Wasm guest conformance.
use atto_compiler::compile;
use atto_compiler::lexer::lex;
use atto_compiler::tuple_binding_lower::lower;

const FIXTURES: &str = r#"
pub fnc fixtures(): Tuple<int,int,int> {
    return (10, 20, 30);
}
"#;

#[test]
fn fixture_call_destructures_and_root_is_let() {
    let src = format!("{FIXTURES}\npub fnc main(): int {{
        let (a, r, mut j) as foo = fixtures();
        j = 8;
        foo = fixtures();
        return a + r + j;
    }}");
    let wasm = compile(&src).expect("scalar tuple return + copy projections must compile");
    assert!(wasm.starts_with(b"\0asm"));
}

#[test]
fn runtime_alias_as_val_is_immutable() {
    let src = format!("{FIXTURES}\npub fnc main(): int {{
        let (a, r, mut j) as val foo = fixtures();
        return a + r + j;
    }}");
    assert!(compile(&src).is_ok());
    let src = format!("{FIXTURES}\npub fnc main(): int {{
        let (a, r, mut j) as val foo = fixtures();
        foo = fixtures();
        return a + r + j;
    }}");
    assert_eq!(compile(&src).unwrap_err().code, "E_IMMUTABLE");
}

#[test]
fn constant_tuple_alias_is_statically_proven() {
    let src = "pub fnc main(): int {
        let (a, const r, mut j) as const foo = (1,2,3);
        j = 5;
        return a + r + j;
    }";
    assert!(compile(src).is_ok());
}

#[test]
fn runtime_const_and_scalar_const_mut_fail_before_code_gen() {
    let runtime = format!("{FIXTURES}\npub fnc main(): int {{
        let (a, r, mut j) as const foo = fixtures();
        return a + r + j;
    }}");
    assert_eq!(compile(&runtime).unwrap_err().code, "E_CONST");
    let scalar = "pub fnc main(): int {
        let (a, const r, const mut j) = (1,2,3);
        return a+r+j;
    }";
    assert_eq!(compile(scalar).unwrap_err().code, "E_CONST_MUT_TARGET");
}

#[test]
fn arity_and_duplicate_names_fail_closed() {
    let shape = "pub fnc main(): int {
        let (a,b,c) = (1,2);
        return 0;
    }";
    assert_eq!(compile(shape).unwrap_err().code, "E_ARITY");
    let duplicate = "pub fnc main(): int {
        let (a,a) as foo = (1,2);
        return 0;
    }";
    assert_eq!(compile(duplicate).unwrap_err().code, "E_DUPLICATE");
}

#[test]
fn bool_float_string_scalar_profiles_lower() {
    let s = r#"
pub fnc fixtures(): Tuple<bool,float,string> { return (true, 3.0, "hi"); }
pub fnc main(): int {
    let (yes, radius, phrase) as val foo = fixtures();
    if yes then
        return 5;
    fi
    return 0;
}
"#;
    assert!(compile(s).is_ok());
}

#[test]
fn plain_non_tuple_sources_and_unsupported_object_tuple_are_gated() {
    let ordinary = "pub fnc main(): int { let n = 10; return n; }";
    assert!(compile(ordinary).is_ok());
    let bad = "pub fnc main(): int { let (a,b) = object(); return 0; }";
    assert_eq!(compile(bad).unwrap_err().code, "E_UNSUPPORTED");
    let object = r#"
define class Cell as pub int value = 1; end
pub fnc fixtures(): Tuple<Cell,int> { return (new Cell(), 1); }
pub fnc main(): int { let (a,b) as foo = fixtures(); return b; }
"#;
    assert_eq!(compile(object).unwrap_err().code, "E_UNSUPPORTED_TYPE");
}

#[test]
fn tuple_annotation_type_is_not_discarded() {
    let ok = "pub fnc main(): int {
        let (a: int, b: bool) = (3, true);
        return a;
    }";
    assert!(compile(ok).is_ok());
    let wrong = "pub fnc main(): int {
        let (a: bool, b: int) = (3, 4);
        return b;
    }";
    assert_eq!(compile(wrong).unwrap_err().code, "E_TYPE");
    let unsupported = "pub fnc main(): int {
        let (a: Array<int>, b) = (3, 4);
        return b;
    }";
    assert_eq!(compile(unsupported).unwrap_err().code, "E_UNSUPPORTED_TYPE");
}

#[test]
fn tuple_return_overloads_are_resolved_by_arity() {
    let bad = r#"
pub fnc fixtures(): Tuple<int,int> { return (1,2); }
pub fnc fixtures(int n): int { return n; }
pub fnc main(): int {
    let (a,b) as foo = fixtures(42);
    return a+b;
}
"#;
    assert_eq!(compile(bad).unwrap_err().code, "E_UNSUPPORTED");
}

#[test]
fn raw_scalar_tuple_tokens_have_deterministic_lowering() {
    let a = lex("pub fnc main(): int { let (a,b) as foo = (5,6); return a+b; }").unwrap();
    let first = lower(a.clone()).unwrap();
    let second = lower(a).unwrap();
    assert_eq!(format!("{first:?}"), format!("{second:?}"));
    assert!(first.iter().any(|t| matches!(&t.tok,
        atto_compiler::lexer::Tok::Ident(name) if name.starts_with("__atto_tuple_private_"))));
}

#[test]
fn tuple_named_local_comparison_remains_an_expression() {
    let source = "pub fnc main(): int {
        let Tuple = 3;
        let is_smaller = Tuple < 7;
        if is_smaller then
            return 1;
        fi
        return 0;
    }";
    assert!(compile(source).is_ok(),
        "identifier named Tuple must not trigger generic type rewriting");
}

#[test]
fn tuple_trailing_comma_and_singleton_constructor_are_canonical() {
    let single = "pub fnc one(): Tuple<int> { return (42,); }
pub fnc main(): int { let (value,) as val root = one(); return value; }";
    assert!(compile(single).is_ok(),
        "one-item Tuple constructor must receive exactly one argument");
    let multi = "pub fnc main(): int {
        let (a,b) = (5, 7,);
        return a+b;
    }";
    assert!(compile(multi).is_ok(),
        "trailing source comma must not leak to the constructor argument list");
}

#[test]
fn controls_in_scalar_tuple_strings_round_trip_exactly() {
    let source = r#"pub fnc main(): int {
        let (s,) as val root = ("\u{0}\u{1f}\n",);
        return 0;
    }"#;
    let original = lex(source).unwrap();
    let expected = String::from("\u{0}\u{1f}\n");
    assert!(original.iter().any(|t|
        matches!(&t.tok, atto_compiler::lexer::Tok::Str(s) if s == &expected)));
    let transformed = lower(original).unwrap();
    assert!(transformed.iter().any(|t|
        matches!(&t.tok, atto_compiler::lexer::Tok::Str(s) if s == &expected)),
        "Atto's string escape semantics must survive tuple lowering");
}

#[test]
fn nonfinite_floats_cannot_be_emitted_as_identifiers() {
    let source = "pub fnc main(): int {
        let (too_big,) = (1e309,);
        return 0;
    }";
    match lex(source) {
        Ok(tokens) => {
            let diagnostic = lower(tokens).unwrap_err();
            assert_eq!(diagnostic.code, "E_FLOAT_LITERAL");
        }
        Err(diagnostic) => assert_eq!(diagnostic.code, "E_FLOAT_LITERAL"),
    }
}

#[test]
fn bounded_monomorphization_limits_distinct_scalar_shapes() {
    let types = ["int", "bool", "float", "string"];
    let literals = ["1", "true", "3.0", "\"a\""];
    let mut source = String::new();
    for i in 0..65 {
        let digits = [i % 4, (i / 4) % 4, (i / 16) % 4, (i / 64) % 4];
        let decl = digits.map(|d| types[d]).join(",");
        let values = digits.map(|d| literals[d]).join(",");
        source.push_str(&format!(
            "pub fnc shape_{i}(): Tuple<{decl}> {{ return ({values}); }}\n"
        ));
    }
    let diagnostic = lower(lex(&source).unwrap()).unwrap_err();
    assert_eq!(diagnostic.code, "E_SOURCE_LIMIT");
}


#[test]
fn tuple_alias_cannot_acquire_exclusive_class_mutation_backdoor() {
    // The tuple root is backed by a compiler-owned class, but it must
    // retain Atto tuple value semantics instead of exposing writable fields.
    for modifier in ["as foo", "as val foo", "as const foo"] {
        let source = format!("pub fnc main(): int {{
            let (a,b) {modifier} = (1,2);
            let loan = borrow! mut foo;
            return a+b;
        }}");
        assert_eq!(compile(&source).unwrap_err().code, "E_BORROW",
            "tuple root {modifier} must not gain mutable class access");
    }
}

#[test]
fn owner_scalar_field_read_rejects_live_exclusive_loan() {
    let source = r#"
define class Cell as
    pub int value: 7;
end
pub fnc main(): int {
    let owner = new Cell();
    let exclusive = borrow! mut owner;
    return owner.value;
}
"#;
    assert_eq!(compile(source).unwrap_err().code, "E_BORROW",
        "read through owner while an exclusive loan exists must fail");
}

#[test]
fn less_than_in_tuple_producer_arguments_preserves_actual_arity() {
    let source = r#"
pub fnc make_pair(bool condition, int n): Tuple<int,int> {
    return (1,n);
}
pub fnc main(): int {
    let (a,b) as val pair = make_pair(1 < 2, 9);
    return a+b;
}
"#;
    assert!(compile(source).is_ok(),
        "a comparison in call arguments must not be treated as a generic type delimiter");
}

#[test]
fn constexpr_element_retains_const_not_runtime_field_provenance() {
    let source = "pub fnc main(): int {
        let (a, const r: int, j) as val foo = (1,2,3);
        return r;
    }";
    let lowered = lower(lex(source).unwrap()).unwrap();
    let mut constant_local_found = false;
    for slice in lowered.windows(4) {
        if matches!((&slice[0].tok, &slice[1].tok, &slice[2].tok, &slice[3].tok),
            (atto_compiler::lexer::Tok::Ident(kind),
             atto_compiler::lexer::Tok::Ident(name),
             atto_compiler::lexer::Tok::Colon,
             atto_compiler::lexer::Tok::Ident(ty))
            if kind == "const" && name == "r" && ty == "int") {
            constant_local_found = true;
        }
    }
    assert!(constant_local_found,
        "const tuple members must remain const after lowering, not become val projections");
    assert!(compile(source).is_ok());

    let rebinding = "pub fnc main(): int {
        let (a, const r, j) = (1,2,3);
        r = 4;
        return r;
    }";
    assert_eq!(compile(rebinding).unwrap_err().code, "E_IMMUTABLE");
}

#[test]
fn decoded_string_byte_quota_cannot_be_bypassed_by_escapes() {
    let many_escapes = "\\n".repeat(atto_compiler::lexer::MAX_STRING_BYTES + 1);
    let source = format!(
        "pub fnc main(): int {{ let (s,) = (\"{many_escapes}\",); return 0; }}"
    );
    let error = compile(&source).unwrap_err();
    assert_eq!(error.code, "E_SOURCE_LIMIT",
        "decoded string escapes must obey MAX_STRING_BYTES");
}

#[test]
fn private_tuple_storage_has_explicit_constructor_write_authority() {
    let source = "pub fnc main(): int {
        let (a,b) as val root = (11, 12);
        return a+b;
    }";
    let tokens = lower(lex(source).unwrap()).unwrap();
    assert!(tokens.windows(4).any(|slice| matches!(
        (&slice[0].tok, &slice[1].tok, &slice[2].tok, &slice[3].tok),
        (atto_compiler::lexer::Tok::Ident(pub_kw),
         atto_compiler::lexer::Tok::Ident(let_kw),
         atto_compiler::lexer::Tok::Ident(ty),
         atto_compiler::lexer::Tok::Ident(field))
         if pub_kw == "pub" && let_kw == "let" && ty == "int" && field == "e0"
    )), "internal constructor-initialized fields must be 'pub let' under default-val class semantics");
    assert!(compile(source).is_ok(), "constructor must initialize private tuple fields");
}
