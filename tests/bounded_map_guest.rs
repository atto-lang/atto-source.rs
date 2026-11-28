use atto_compiler::{compile, compile_js};

#[test]
fn map_factory_emits_both_targets() {
    for source in [
        "fnc main(): int { let n = 40; let m = Map.of(Entry.of(-7,n+2),Entry.of(3,9)); return m.get_or(-7,0); }",
        "fnc main(): int { let m = Map.of(Entry.of(1,2)); let n = m.len(); return m.contains_key(1) ? n : 0; }",
    ] {
        assert_eq!(&compile(source).unwrap()[..4], b"\0asm");
        assert!(compile_js(source).is_ok());
    }
}

#[test]
fn map_factory_rejects_unproven_key_identity_and_escapes() {
    for source in [
        "fnc main(): int { let m = Map.of(Entry.of(0,1),Entry.of(-0,2)); return m.len(); }",
        "fnc main(): int { let n = 1; let m = Map.of(Entry.of(n,2)); return m.len(); }",
        "fnc main(): int { let m = Map.of(Entry.of(1,true)); return m.len(); }",
        "fnc main(): int { let m = Map.of(Entry.of(1,2)); return m.get_or(true,0); }",
        "fnc main(): int { let m = Map.of(Entry.of(1,2)); return m; }",
        "fnc main(): int { let m = Map.of(Entry.of(1,2)); let a = m; return a.len(); }",
        "fnc main(): int { let m = Map.of(Entry.of(1,2)); m.insert(3,4); return m.len(); }",
        "fnc main(): int { let Map = 1; let m = Map.of(Entry.of(1,2)); return m.len(); }",
    ] {
        assert!(compile(source).is_err(), "{source}");
        assert!(compile_js(source).is_err(), "{source}");
    }
}
