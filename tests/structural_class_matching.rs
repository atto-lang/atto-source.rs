//! Compiler-backed contract for the bounded class-only structural value
//! matcher. Native-host compilation verifies a real Wasm binary, but is not
//! itself proof that a guest VM executed it. See conformance.mjs for that.
use atto_compiler::compile;

const CLASS: &str = r#"
define class Point as
    pub int x = 10;
    pub int y = 32;
end
"#;

fn source(body: &str) -> String {
    format!("{CLASS}\npub fnc main(): int {{\n{body}\n}}")
}

#[test]
fn class_structural_match_generates_deterministic_real_wasm() {
    let samples = [
        r#"val v = new Point(); match v over
              on ~{ x?: 10, y: _ } as val p -> {
                  return p.x + p.y;
              }
              on _ -> { return 0; }
            end"#,
        r#"val v = new Point(); match v over
              on ~ { x?: 10, y: _ } as val p ->
                  return p.x + p.y;
              end
              on _ -> { return 0; }
            end"#,
        r#"val v = new Point(); match v over
              on structural { x?: 10, y: _ } as val p -> {
                  return p.x + p.y;
              }
              on _ -> { return 0; }
            end"#,
        r#"val v = new Point(); match v over
              on ~ { x?: 11, y: _ } -> { return 1; }
              on _ -> { return 42; }
            end"#,
    ];
    for sample in samples {
        let src = source(sample);
        let wasm = compile(&src).unwrap_or_else(|d| panic!("{src}: {d}"));
        assert_eq!(&wasm[..8], b"\0asm\x01\0\0\0");
        assert_eq!(wasm, compile(&src).unwrap(), "Wasm must be deterministic");
    }
}

#[test]
fn class_match_rejects_private_field_access_and_escaping_borrow() {
    for (src, code) in [
        (
            r#"define class Hidden as
                 int x = 10;
                 pub int y = 32;
               end
               pub fnc main(): int {
                 val v = new Hidden();
                 match v over
                   on ~{ x?: 10, y: _ } -> { return 42; }
                   on _ -> { return 0; }
                 end
               }"#,
            "E_PRIVATE",
        ),
        (
            r#"define class Point as
                 pub int x = 10;
                 pub int y = 32;
               end
               fnc leak(): Point {
                 val v = new Point();
                 match v over
                   on ~{ x?: 10, y: _ } as val p -> { return p; }
                   on _ -> { return v; }
                 end
               }
               pub fnc main(): int { return 42; }"#,
            "E_BORROW",
        ),
        (
            r#"define class Point as
                 pub int x = 10;
               end
               pub fnc main(): int {
                 val v = new Point();
                 match v over
                   on ~{ x?: 10 } -> { return 42; }
                 end
               }"#,
            "E_MATCH",
        ),
        (
            r#"define class Point as
                 pub int x = 10;
               end
               pub fnc main(): int {
                 val v = new Point();
                 match v over
                   on is ~UnknownPoint as val p -> { return 42; }
                   on _ -> { return 0; }
                 end
               }"#,
            "E_UNKNOWN_TYPE",
        ),
    ] {
        let d = compile(src).expect_err("unproven class pattern must fail closed");
        assert_eq!(d.code, code, "{src}: {d}");
    }
}

#[test]
fn non_class_subjects_cannot_use_structural_field_value_patterns() {
    for source in [
        "pub fnc main(): int { match 4 over on ~{ x?: 10 } -> { return 1; } on _ -> { return 42; } end }",
        "pub fnc main(): int { match true over on ~{ x?: 10 } -> { return 1; } on _ -> { return 42; } end }",
    ] {
        let d = compile(source).expect_err("class-only capability must reject other types");
        assert_eq!(d.code, "E_UNSUPPORTED", "{source}: {d}");
    }
}
