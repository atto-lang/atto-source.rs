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
        if let Some(col) = header(&src, "expect-column") {
            assert_eq!(d.pos.col.to_string(), col, "{name}");
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

#[test]
fn long_operator_and_elif_chains_fail_closed_instead_of_overflowing() {
    let chain = |n: usize, op: &str| format!("pub fnc main(): int {{ return {}; }}", vec!["1"; n].join(op));
    assert!(atto_compiler::compile(&chain(250, " + ")).is_ok());
    for op in [" + ", " * ", " - ", " == ", " < "] {
        assert_eq!(atto_compiler::compile(&chain(3_000, op)).unwrap_err().code, "E_NESTING", "{op}");
    }
    let elif = |n: usize| {
        let arms = (1..n)
            .map(|i| format!("elif x eq {i} then return {i};"))
            .collect::<Vec<_>>()
            .join("\n");
        format!("pub fnc main(): int {{\nval x = 1;\nif x eq 0 then return 0;\n{arms}\nfi\nreturn 0;\n}}")
    };
    assert!(atto_compiler::compile(&elif(250)).is_ok());
    assert_eq!(atto_compiler::compile(&elif(300)).unwrap_err().code, "E_NESTING");
}
