//! Native-host checks over the shared fixture corpus (the Node harness runs
//! the same corpus through the wasm32 build and executes the guests).
use std::fs;

fn header<'a>(src: &'a str, key: &str) -> Option<&'a str> {
    let prefix = format!("// {key}: ");
    src.lines().find_map(|l| l.strip_prefix(prefix.as_str()))
}

fn corpus(kind: &str) -> Vec<(String, String)> {
    let mut v: Vec<_> = fs::read_dir(format!("tests/fixtures/{kind}"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "atto"))
        .map(|p| (p.display().to_string(), fs::read_to_string(&p).unwrap()))
        .collect();
    v.sort();
    v
}

#[test]
fn positive_fixtures_compile_to_valid_looking_modules() {
    let c = corpus("pass");
    assert!(c.len() >= 25);
    for (name, src) in c.iter().chain(corpus("panic").iter()) {
        let bytes = atto_compiler::compile(src).unwrap_or_else(|d| panic!("{name}: {d}"));
        assert_eq!(&bytes[..8], b"\0asm\x01\0\0\0", "{name}");
        assert_eq!(atto_compiler::compile(src).unwrap(), bytes, "{name}: deterministic");
    }
}

#[test]
fn negative_fixtures_fail_closed_with_expected_code() {
    let c = corpus("fail");
    assert!(c.len() >= 25);
    for (name, src) in c {
        let want = header(&src, "expect-error").unwrap_or_else(|| panic!("{name}: no expect-error"));
        let d = atto_compiler::compile(&src).expect_err(&name);
        assert_eq!(d.code, want, "{name}: {d}");
        if let Some(line) = header(&src, "expect-line") {
            assert_eq!(d.pos.line.to_string(), line, "{name}");
        }
    }
}

#[test]
fn diagnostics_carry_line_and_column() {
    let d = atto_compiler::compile("pub fnc main(): int {\n  val x = 1;\n  x = 2;\n  return x;\n}").unwrap_err();
    assert_eq!((d.code, d.pos.line, d.pos.col), ("E_IMMUTABLE", 3, 3));
    let d = atto_compiler::compile("pub fnc main(): int {\n  return y;\n}").unwrap_err();
    assert_eq!((d.code, d.pos.line, d.pos.col), ("E_UNBOUND", 2, 10));
}

#[test]
fn source_limit_is_enforced() {
    let big = format!("pub fnc main(): int {{ return 0; }}{}", " ".repeat(atto_compiler::SOURCE_CAP));
    assert_eq!(atto_compiler::compile(&big).unwrap_err().code, "E_SOURCE_LIMIT");
}

fn long_chain(n: usize, op: &str) -> String {
    format!("pub fnc main(): int {{ return {}; }}", vec!["1"; n].join(op))
}

fn long_elif(n: usize) -> String {
    let arms = (1..n)
        .map(|i| format!("elif x eq {i} then return {i};"))
        .collect::<Vec<_>>()
        .join("\n");
    format!("pub fnc main(): int {{\nval x = 1;\nif x eq 0 then return 0;\n{arms}\nfi\nreturn 0;\n}}")
}

#[test]
fn bounded_additive_chain_compiles_without_stack_overflow() {
    assert!(atto_compiler::compile(&long_chain(250, " + ")).is_ok());
}

#[test]
fn oversized_operator_chains_fail_closed_with_nesting_diagnostic() {
    for op in [" + ", " * ", " - ", " == ", " < "] {
        assert_eq!(atto_compiler::compile(&long_chain(3_000, op)).unwrap_err().code, "E_NESTING", "{op}");
    }
}

#[test]
fn bounded_elif_chain_compiles_without_stack_overflow() {
    assert!(atto_compiler::compile(&long_elif(250)).is_ok());
}

#[test]
fn oversized_elif_chain_fails_closed_with_nesting_diagnostic() {
    assert_eq!(atto_compiler::compile(&long_elif(300)).unwrap_err().code, "E_NESTING");
}
