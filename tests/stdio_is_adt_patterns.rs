//! ADT constructor-shaped patterns use a typed, closed set of variants.
//! Angle bracket arguments bind payloads; they are not generic type tests.
use atto_compiler::compile;

const MAIN: &str = "pub fnc main(): int { return 0; }";
fn source(arm: &str) -> String {
    format!(r#"
        pub susp fnc probe(Channel<int> c): int {{
            match c.try_send(41) over
                {arm}
                on is Err<error> -> {{ return 9; }}
            end
        }}
        {MAIN}
    "#)
}
fn expect_wasm(source: &str) {
    let binary=compile(source).unwrap_or_else(|err|panic!("compile: {err}: {source}"));
    assert!(binary.starts_with(b"\0asm\x01\0\0\0"));
}

#[test]
fn is_ok_discard_matches_unit_result() {
    for arm in ["on Ok(_) -> { return 41; }",
                "on Ok -> { return 41; }",
                "on is Ok<_> -> { return 41; }",
                "on is Ok -> { return 41; }"] {
        expect_wasm(&source(arm));
    }
}

#[test]
fn is_result_and_option_payloads_are_typed_as_bindings() {
    let source=format!(r#"
        pub susp fnc probe(Channel<int> c): int {{
            return match << c over
                on is Err<error> -> 7,
                on is Ok<option> -> match option over
                    on is Some<item> -> item,
                    on is None -> 0,
                end,
            end;
        }}
        {MAIN}
    "#);
    expect_wasm(&source);
}

#[test]
fn exact_user_nested_stdio_expression_is_source_compilable() {
    let source=r#"
        pub suspends fnc read_checked_echo(): string using io, suspend {
          val input = stdio.in;
          val output = stdio.out;
          val prompted = writech output, "prompt";
          return match prompted over
            on is Ok<_> -> match << input over
              on is Ok<line> -> match line over
                on is Some<item> -> item,
                on None -> "",
              end,
              on is Err<error> -> "",
            end,
            on is Err<error> -> "",
          end;
        }
        pub fnc main(): int { return 0; }
    "#;
    expect_wasm(source);
    expect_wasm(&source.replace("match << input over","match readch input over"));
}

#[test]
fn malformed_extractors_and_wrong_adt_variants_fail_without_artifacts() {
    for (s,diagnostic) in [
        (source("on is None<_> -> { return 0; }"),"E_ARITY"),
        (source("on is Some<x> -> { return x; }"),"E_TYPE"),
        (source("on is Err<> -> { return 0; }"),"E_SYNTAX"),
        (source("on is Ok<x, y> -> { return 0; }"),"E_SYNTAX"),
        (source("on is Ok<Some<x>> -> { return 0; }"),"E_SYNTAX"),
        (source("on is ArbitraryExtractor<x> -> { return 0; }"),"E_PATTERN"),
        (source("on is Ok<value> -> { return 0; }"),"E_ARITY"),
    ] {
        let diagnostic_actual=compile(&s).expect_err("unverified extractor must not emit Wasm");
        assert_eq!(diagnostic_actual.code,diagnostic,"{s}: {diagnostic_actual}");
    }
}

#[test]
fn payload_is_not_silently_erased_from_result_option() {
    let s=format!(r#"
        pub susp fnc probe(Channel<int> c): int {{
            match << c over
                on is Ok -> {{ return 0; }}
                on is Err<error> -> {{ return 9; }}
            end
        }}
        {MAIN}
    "#);
    let diag=compile(&s).expect_err("bare Ok cannot unwrap Result<Option<int>,IoError>");
    assert_eq!(diag.code,"E_ARITY");
}
