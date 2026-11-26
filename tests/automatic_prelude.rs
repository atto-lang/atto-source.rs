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
    assert!(wasm.starts_with(b"\0asm\x01\0\0\0"));
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

#[test]
fn core_ownership_categories_do_not_grant_unimplemented_borrow_lowering() {
    use prelude::Ownership;
    assert_eq!(prelude::ownership("Option"), Some(Ownership::PayloadDependent));
    assert_eq!(prelude::ownership("Result"), Some(Ownership::PayloadDependent));
    assert_eq!(prelude::ownership("Some"), Some(Ownership::PayloadDependent));
    assert_eq!(prelude::ownership("Slice"), Some(Ownership::BorrowedView));
    assert_eq!(prelude::ownership("Map"), Some(Ownership::Owned));
    assert_eq!(prelude::ownership("Box"), Some(Ownership::Owned));
    assert_eq!(prelude::ownership("Eq"), Some(Ownership::Trait));
    assert_eq!(prelude::ownership("Object"), None);
    assert!(prelude::core_sum_carrier("Option", 1));
    assert!(prelude::core_sum_carrier("Result", 2));
    assert!(!prelude::core_sum_carrier("Option", 2));
    assert!(!prelude::core_sum_carrier("Result", 1));
    assert!(!prelude::core_sum_carrier("Object", 1));
    assert!(prelude::protected_identity("int"));
    // From is catalog-only in this backend, not a reserved lowered identity.
    assert!(!prelude::protected_identity("From"));
    assert_eq!(prelude::type_info("From").unwrap().lowering, prelude::LoweringStatus::ContractOnly);
    for primitive in ["int", "i64", "bool", "float", "string"] {
        assert!(prelude::protected_identity(primitive), "{primitive}");
    }
    assert!(prelude::protected_identity("Option"));
    assert!(!prelude::protected_identity("HashMap"));
}

#[test]
fn canonical_type_registry_pins_arity_ownership_allocation_drop_and_send() {
    use prelude::{Allocation, DropPolicy, LoweringStatus, Ownership, SendPolicy};

    for binding in prelude::BINDINGS {
        let info = prelude::type_info(binding.name).expect(binding.name);
        assert_eq!(info.canonical, binding.canonical);
        assert_eq!(info.kind, binding.kind);
        assert_eq!(info.scope, binding.scope);
        assert!(!info.canonical.contains(".."));
        assert!(info.accepts_arity(info.min_generic_arity as usize)
            || info.kind == prelude::Kind::Trait);
    }

    let int = prelude::type_info("int").unwrap();
    assert_eq!(int.canonical, "core/primitive/i64");
    assert_eq!(int.ownership, Ownership::Copy);
    assert_eq!(int.send_policy, SendPolicy::Always);
    assert_eq!(int.lowering, LoweringStatus::WasmPrimitive);
    assert_eq!(int.canonical, prelude::type_info("i64").unwrap().canonical);
    assert_eq!(prelude::type_info("bool").unwrap().lowering, LoweringStatus::WasmPrimitive);
    assert_eq!(prelude::type_info("string").unwrap().ownership, Ownership::BorrowedView);

    let option = prelude::type_info("Option").unwrap();
    assert_eq!(option.min_generic_arity, 1);
    assert_eq!(option.max_generic_arity, 1);
    assert_eq!(option.ownership, Ownership::PayloadDependent);
    assert_eq!(option.allocation, Allocation::None);
    assert_eq!(option.drop_policy, DropPolicy::ActivePayloadOnce);
    assert_eq!(option.send_policy, SendPolicy::PayloadDependent);

    let result = prelude::type_info("Result").unwrap();
    assert_eq!(result.min_generic_arity, 2);
    assert_eq!(result.max_generic_arity, 2);
    assert_eq!(result.drop_policy, DropPolicy::ActivePayloadOnce);

    assert_eq!(prelude::type_info("None").unwrap().drop_policy, DropPolicy::None);
    for name in ["Array", "FixedArray", "Tuple", "Pair", "Entry"] {
        assert_eq!(prelude::type_info(name).unwrap().drop_policy,
            DropPolicy::AllInitializedMembers, "{name}");
    }

    let slice = prelude::type_info("Slice").unwrap();
    assert_eq!(slice.ownership, Ownership::BorrowedView);
    assert_eq!(slice.allocation, Allocation::BorrowedView);
    assert_eq!(slice.send_policy, SendPolicy::NeverBorrowed);

    let list = prelude::type_info("List").unwrap();
    assert_eq!(list.scope, Scope::Alloc);
    assert_eq!(list.allocation, Allocation::AllocatorRequired);
    assert_eq!(list.drop_policy, DropPolicy::OwnedStorage);
    assert_eq!(list.lowering, LoweringStatus::ContractOnly);

    let string = prelude::type_info("String").unwrap();
    assert_eq!(string.ownership, Ownership::BorrowedView);
    assert_eq!(string.lowering, LoweringStatus::LegacyStringView);
    assert_eq!(string.drop_policy, DropPolicy::None);

    assert!(prelude::type_info("HashMap").unwrap().scope == Scope::Explicit);
    assert!(prelude::type_info("Object").is_none());
    assert!(!option.accepts_arity(2));
    assert!(!result.accepts_arity(1));
    assert!(!prelude::core_sum_carrier("Option", 2));
    assert!(!prelude::core_sum_carrier("Result", 1));
}
