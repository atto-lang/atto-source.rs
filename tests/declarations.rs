//! The browser frontend admits only inert top-level define declarations.
use atto_compiler::compile;

fn source(definition: &str) -> String {
    format!("{definition}\npub fnc main(): int {{ return 42; }}")
}

#[test]
fn inert_declarations_compile_to_guest_wasm() {
    for d in [
        "define class Point as end",
        "define actor Worker as end",
        "pub define class Point {}",
        "define pub actor Worker as end",
        "define enum Status as Ready, Running end",
        "define enum Status as\n Ready\n Running\nend",
    ] {
        let wasm = compile(&source(d)).unwrap_or_else(|e| panic!("{d}: {e}"));
        assert_eq!(&wasm[..8], b"\0asm\x01\0\0\0");
    }
}

#[test]
fn executable_static_class_methods_compile_to_distinct_real_wasm() {
    let with_class = "define class Calculator as\n  static fnc add(int a, int b): int { return a + b; }\nend\npub fnc main(): int { return Calculator.add(20, 22); }";
    let without_class = "pub fnc main(): int { return 42; }";
    let wasm = compile(with_class).expect("static method compiled");
    assert_eq!(&wasm[..8], b"\0asm\x01\0\0\0");
    assert_ne!(wasm, compile(without_class).expect("standalone main"));
}

#[test]
fn member_semantics_and_collisions_reject_before_codegen() {
    for (d, expected) in [
        ("define class P as\nint x: 0\nend", "E_UNSUPPORTED"),
        ("define actor A as\nreceive(m) {}\nend", "E_UNKNOWN_TYPE"),
        ("define enum E as Red = 1 end", "E_UNSUPPORTED"),
        ("define enum E as Red, Red end", "E_DUPLICATE"),
        ("define class P as end\ndefine actor P as end", "E_DUPLICATE"),
        ("define enum E as end", "E_SYNTAX"),
        ("define actor A as", "E_EOF"),
        ("define module M as end", "E_UNSUPPORTED"),
        ("define class P as fnc get(): int { return 1; } end", "E_UNSUPPORTED"),
        ("define class P as static fnc get(): int { return 1; } static fnc get(): int { return 2; } end", "E_DUPLICATE"),
        ("define class P as static fnc get(): int { return 1; } end\nfnc P(): int { return 1; }", "E_DUPLICATE"),
    ] {
        let input = if d == "define actor A as" { d.to_string() } else { source(d) };
        let err = compile(&input).expect_err(d);
        assert_eq!(err.code, expected, "{d}: {err}");
    }
}

#[test]
fn no_false_runtime_support_for_enum_or_spawn() {
    for s in [
        "define enum E as Red end\npub fnc main(): int { return E.Red; }",
        "define actor A as end\npub fnc main(): int { spawn A(); return 42; }",
        "pub fnc main(): int { define class P as end\nreturn 42; }",
    ] {
        assert!(compile(s).is_err(), "{s}");
    }
}
