//! Automatic prelude catalog and real compiler admission tests.
//! These tests do not assert that unlowered alloc types are executable.

use atto_compiler::{compile, prelude::{self, Kind, Profile, Scope}};

#[test]
fn canonical_core_names_are_implicitly_in_scope() {
    for (short, canonical) in [
        ("Option", "core/option/Option"),
        ("Result", "core/result/Result"),
        ("Some", "core/option/Some"),
        ("None", "core/option/None"),
        ("Ok", "core/result/Ok"),
        ("Err", "core/result/Err"),
        ("Array", "core/array/Array"),
        ("Pair", "core/tuple/Pair"),
    ] {
        let entry = prelude::resolve(short, Profile::Core).expect(short);
        assert_eq!(entry.canonical, canonical);
        assert_eq!(entry.scope, Scope::Core);
    }
    assert_eq!(prelude::resolve("Some", Profile::Core).unwrap().kind, Kind::Constructor);
}

#[test]
fn alloc_scope_requires_an_allocator_profile_and_explicit_names_stay_explicit() {
    for name in ["String", "List", "Vec", "Map", "Set", "Box"] {
        assert!(prelude::resolve(name, Profile::Core).is_none(), "{name}");
        assert_eq!(prelude::resolve(name, Profile::Alloc).unwrap().scope, Scope::Alloc, "{name}");
        assert_eq!(prelude::resolve(name, Profile::Hosted).unwrap().scope, Scope::Alloc, "{name}");
    }
    for name in ["HashMap", "HashSet", "Entry"] {
        assert!(prelude::resolve(name, Profile::Core).is_none(), "{name}");
        assert!(prelude::resolve(name, Profile::Alloc).is_none(), "{name}");
        assert!(prelude::resolve(name, Profile::Hosted).is_none(), "{name}");
        assert_eq!(prelude::BINDINGS.iter().find(|b| b.name == name).unwrap().scope, Scope::Explicit);
    }
    assert!(prelude::resolve("Object", Profile::Hosted).is_none());
}

#[test]
fn catalog_has_unique_spellings_and_no_accidental_hosted_side_effect_bindings() {
    let mut names = std::collections::BTreeSet::new();
    for b in prelude::BINDINGS {
        assert!(names.insert(b.name), "duplicate prelude name {}", b.name);
        assert!(!b.canonical.contains(".."));
        assert!(!b.canonical.starts_with('/'));
        assert_eq!(b.canonical.rsplit('/').next(), Some(b.name));
    }
}

#[test]
fn real_option_and_result_variants_compile_without_imports() {
    // This source is already pinned by a successful compiler fixture.
    let src = include_str!("playground_previews/result-option-end.atto");
    let wasm = compile(src).expect("builtin core ADTs must compile without explicit imports");
    assert!(wasm.starts_with(b"\\0asm\\x01\\0\\0\\0"));
    assert_eq!(compile(src).unwrap(), wasm, "deterministic Wasm");
}

#[test]
fn spoofed_builtin_variants_are_rejected() {
    for name in ["Some", "None", "Ok", "Err", "Option", "Result"] {
        let source = format!(
            "define class {name} as\n  mut int n = 0;\nend\npub fnc main(): int {{ return 42; }}"
        );
        let error = compile(&source).expect_err(name);
        assert_eq!(error.code, "E_DUPLICATE", "{name}: {error}");
    }
}

#[test]
fn prelude_catalog_does_not_turn_unsupported_alloc_types_into_guest_storage() {
    for name in ["List<int>", "Map<int,int>", "Box<int>"] {
        let source = format!("pub fnc main(): int {{ val x: {name} = 0; return 42; }}");
        assert!(compile(&source).is_err(), "{name} must fail closed until verified lowering");
    }
}
