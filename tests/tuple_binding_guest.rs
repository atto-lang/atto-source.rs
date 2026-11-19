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
    assert!(wasm.starts_with(b"\\0asm"));
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
fn raw_scalar_tuple_tokens_have_deterministic_lowering() {
    let a = lex("pub fnc main(): int { let (a,b) as foo = (5,6); return a+b; }").unwrap();
    let first = lower(a.clone()).unwrap();
    let second = lower(a).unwrap();
    assert_eq!(format!("{first:?}"), format!("{second:?}"));
    assert!(first.iter().any(|t| matches!(&t.tok,
        atto_compiler::lexer::Tok::Ident(name) if name.starts_with("__atto_tuple_private_"))));
}
