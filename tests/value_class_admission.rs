//! Stage A admission only: this suite MUST NOT be cited as guest support.
use atto_compiler::{compile, lexer::lex, value_class_admission};

fn expect_fail(source: &str) {
    let d = compile(source).expect_err("value class without value ABI must fail closed");
    assert_eq!(d.code, "E_VALUE_UNSUPPORTED", "{d}");
    assert!(d.pos.line > 0 && d.pos.col > 0, "{d}");
}

#[test]
fn value_class_headers_fail_closed_without_emitting_wasm() {
    for header in [
        "define value class Point",
        "define pub value class Point",
        "pub define value class Point",
    ] {
        expect_fail(&format!(
            "{header} as pub final int x; end\npub fnc main(): int {{ return 42; }}"
        ));
    }
}

#[test]
fn value_class_not_silently_rewritten_to_regular_class() {
    expect_fail(r#"
        define value class Point as
            pub final int x;
            constructor(int x) { self.x = x; }
        end
        pub fnc main(): int {
            val Point a = Point(42);
            val Point b = Point(42);
            if a == b then return 42; else return 0; fi
        }
    "#);
}

#[test]
fn comments_and_string_literals_are_not_value_declarations() {
    let s = r#"
        // define value class Fake as end
        pub fnc main(): int {
            stdio.println("define value class Fake as end");
            return 42;
        }
    "#;
    value_class_admission::preflight(&lex(s).unwrap()).unwrap();
    let guest = compile(s).unwrap();
    assert!(guest.starts_with(b"\0asm\x01\0\0\0"));
}

#[test]
fn ordinary_class_and_struct_syntax_stays_admitted() {
    let s = r#"
        define struct Pair as int a; int b; end
        pub fnc main(): int { val p = Pair{a: 20, b: 22}; return p.a + p.b; }
    "#;
    value_class_admission::preflight(&lex(s).unwrap()).unwrap();
    let guest = compile(s).unwrap();
    assert!(guest.starts_with(b"\0asm\x01\0\0\0"));
}
