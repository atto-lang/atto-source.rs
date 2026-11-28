use atto_compiler::{compile, compile_js};

#[test]
fn admitted_profiles_emit_both_guest_targets() {
    for source in [
        "fnc main(): int { let s = Set.of(7, 7, 42); return s.len(); }",
        "fnc main(): int { let n = -7; let s = Set.of(n, n + 49, n); return s.contains(42) ? 42 : 0; }",
    ] {
        assert_eq!(&compile(source).unwrap()[..4], b"\0asm");
        assert!(compile_js(source).is_ok());
    }
}

#[test]
fn unsupported_escapes_and_types_fail_closed() {
    for source in [
        "fnc main(): int { let s = Set.of(1); return s; }",
        "fnc main(): int { let s = Set.of(1); let alias = s; return alias.len(); }",
        "fnc main(): int { let s = Set.of(1); s.add(2); return s.len(); }",
        "fnc main(): int { let s = Set.of(1, true); return s.len(); }",
        "fnc main(): int { let s = Set.of(1); return s.contains(true) ? 1 : 0; }",
        "fnc main(): int { let s = Set.of(1,2,3,4,5,6,7,8,9); return s.len(); }",
        "fnc main(): int { let Set = 1; let s = Set.of(1); return s.len(); }",
    ] {
        assert!(compile(source).is_err(), "{source}");
        assert!(compile_js(source).is_err(), "{source}");
    }
}
