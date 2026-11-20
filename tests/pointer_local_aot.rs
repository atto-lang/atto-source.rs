//! Real native Rust compiler regressions for the bounded nonallocating local
//! Pointer<int> profile. Wasm32 compiler and guest execution are tested by
//! tests/conformance.mjs over the same pass fixture files.
const RT: &str = r#"
pub fnc main(): int {
  let value: int = 42;
  val Pointer<int> ptr = rt ref value;
  val answer = rt deref ptr;
  rt unref ptr;
  return answer;
}
"#;
const MIXED: &str = r#"
pub fnc main(): int {
  let value: int = 42;
  val Pointer<int> ptr = rt ref value;
  val answer = deref! ptr;
  unref! ptr;
  return answer;
}
"#;

fn compile_ok(src: &str) -> Vec<u8> {
    let bytes = atto_compiler::compile(src).unwrap_or_else(|e| panic!("{e}: {src}"));
    assert_eq!(&bytes[..8], b"\0asm\x01\0\0\0");
    assert_eq!(bytes, atto_compiler::compile(src).unwrap());
    bytes
}
fn reject(src: &str) {
    let d = atto_compiler::compile(src).expect_err("unsafe Pointer<int> source emitted a guest");
    assert!(d.code.starts_with("E_POINTER_"), "{d}");
}
#[test]
fn both_exact_user_programs_compile_to_deterministic_equivalent_guest_wasm() {
    let a = compile_ok(RT);
    let b = compile_ok(MIXED);
    assert_eq!(a, b, "aliases must produce byte-identical compiled guest modules");
    let scalar = r#"pub fnc main(): int { let value: int = 42; val answer = value; return answer; }"#;
    assert_eq!(a, compile_ok(scalar), "proven local pointer must lower to normal checked scalar ABI");
}
#[test]
fn all_canonical_and_legacy_prefix_pairs_have_same_guest_effect() {
    for reference in ["rt ref value", "ref! value", "ref!value"] {
        for read in ["rt deref ptr", "deref! ptr", "deref!ptr"] {
            for release in ["rt unref ptr", "unref! ptr", "unref!ptr"] {
                let s = format!(r#"pub fnc main(): int {{
                    let value: int = 42;
                    val Pointer<int> ptr = {reference};
                    val answer = {read};
                    {release};
                    return answer;
                }}"#);
                assert_eq!(compile_ok(&s), compile_ok(RT), "{reference}/{read}/{release}");
            }
        }
    }
}
#[test]
fn canonical_prl_001_and_prl_002_unmodified_source_compile_as_real_guests() {
    // Source bytes are Git-blob-identical to atto-platform-conformance #30.
    // These files have NO compiler-side rewritten comments or ownership ops.
    const PRL_001: &str = include_str!("pointer_local_spec_exact/PRL-001.atto");
    const PRL_002: &str = include_str!("pointer_local_spec_exact/PRL-002.atto");
    let guest_a = compile_ok(PRL_001);
    let guest_b = compile_ok(PRL_002);
    assert_eq!(guest_a, guest_b, "rt and bang spellings have equivalent guest code");
    assert_eq!(guest_a, compile_ok(RT));
    assert_eq!(guest_b, compile_ok(MIXED));
}

#[test]
fn malformed_spellings_and_pointer_lifecycle_fail_before_guest_generation() {
    for body in [
        "val Pointer<int> ptr = rt ref value; val answer = deref! ptr; return answer;",
        "val Pointer<int> ptr = rt ref value; val answer = deref! ptr; rt unref ptr; rt unref ptr; return answer;",
        "val Pointer<int> ptr = rt ref value; val answer = rt deref ptr; rt unref ptr; return ptr;",
        "val Pointer<int> ptr = rt ref value; val answer = deref !ptr; rt unref ptr; return answer;",
        "val Pointer<int> ptr = rt ref value; val answer = deref! ptr; unref !ptr; return answer;",
        "val Pointer<int> ptr = rt ref (40 + 2); val answer = deref! ptr; rt unref ptr; return answer;",
        "val Pointer<int> ptr = rt ref value; val Pointer<int> duplicate = ptr; val answer = deref! ptr; rt unref ptr; return answer;",
        "val Pointer<int> ptr = rt ref value; val answer = deref! ptr; ptr.close(); rt unref ptr; return answer;",
        "val Pointer<int> ptr = rt ref value; val answer = deref! ptr; ptr.write_at(0, 99); rt unref ptr; return answer;",
        "val Pointer<int> ptr = rt ref value; val answer = deref! ptr; value = 43; rt unref ptr; return answer;",
        "val Pointer<int> ptr = rt ref value; val answer = deref! ptr; rt unref ptr; value = 43; return answer;",
        "val Pointer<bool> ptr = rt ref value; val answer = deref! ptr; rt unref ptr; return answer;",
    ] {
        let src = format!("pub fnc main(): int {{ let value: int = 42; {body} }}");
        reject(&src);
    }
}
#[test]
fn uninitialized_locals_cross_domain_and_early_exits_fail_closed() {
    for src in [
        "pub fnc main(): int { let value: int; val Pointer<int> ptr = rt ref value; val answer = deref! ptr; rt unref ptr; return answer; }",
        "pub fnc main(): int { let value: int = 42; val Pointer<int> ptr = rt ref value; val answer = deref! ptr; return answer; rt unref ptr; }",
        "pub fnc main(): int { let value: int = 42; val Pointer<int> ptr = rt ref value; val answer = deref! ptr; do if true then rt unref ptr; fi return answer; }",
        "pub fnc main(): int { let value: int = 42; val Pointer<int> ptr = rt ref value; val answer = deref! ptr; stdio.println(answer); rt unref ptr; return answer; }",
        "pub fnc main(): int { let value: int = 42; val Pointer<int> ptr = rt ref value; val answer = deref! ptr; rt unref ptr; return answer; } pub fnc other(): int { return 7; }",
    ] { reject(src); }
}
