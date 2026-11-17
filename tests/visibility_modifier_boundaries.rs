//! Compiler-side admission and rejection for external/internal/protected/final.
//! No foreign linker, subclass checker or cross-package runtime is claimed.
use atto_compiler::compile;

fn reject(source: &str, code: &str) {
    let err = compile(source).expect_err("unsafe syntax must not produce a guest");
    assert_eq!(err.code, code, "{source}: {err}");
    assert!(err.pos.line > 0 && err.pos.col > 0);
}

#[test]
fn internal_functions_stay_local_and_cannot_be_exported() {
    let src = "internal fnc secret(): int { return 41; }\npub fnc main(): int { return secret() + 1; }";
    assert!(compile(src).is_ok(), "internal function must remain callable in its source");
    reject(
        "export pub fnc secret;\ninternal fnc secret(): int { return 42; }\npub fnc main(): int { return 0; }",
        "E_VISIBILITY",
    );
    reject(
        "internal fnc secret(): int { return 42; }\nexport pub[pkg] secret;\npub fnc main(): int { return 0; }",
        "E_VISIBILITY",
    );
    reject(
        "internal pub fnc secret(): int { return 42; }\npub fnc main(): int { return 0; }",
        "E_VISIBILITY",
    );
    reject(
        "pub internal fnc secret(): int { return 42; }\npub fnc main(): int { return 0; }",
        "E_VISIBILITY",
    );
}

#[test]
fn external_prototypes_never_authorize_unvalidated_calls_or_exports() {
    reject(
        "external fnc host_clock_ns(): int;\npub fnc main(): int { return host_clock_ns(); }",
        "E_UNSUPPORTED",
    );
    reject(
        "export pub fnc host_clock_ns;\nexternal fnc host_clock_ns(): int;\npub fnc main(): int { return 0; }",
        "E_UNBOUND",
    );
    reject(
        "pub external fnc host_clock_ns(): int;\npub fnc main(): int { return 0; }",
        "E_VISIBILITY",
    );
    reject(
        "external fnc clock(): int;\nexternal fnc clock(): int;\npub fnc main(): int { return 0; }",
        "E_DUPLICATE",
    );
}

#[test]
fn protected_must_not_be_silently_rewritten_to_private() {
    reject(
        "define class C as\n protected final int id: 7;\nend\npub fnc main(): int { return 0; }",
        "E_UNSUPPORTED",
    );
    reject(
        "define class C as\n protected let int balance = 7;\nend\npub fnc main(): int { return 0; }",
        "E_UNSUPPORTED",
    );
}

#[test]
fn final_field_initializer_is_not_a_second_assignment() {
    let source = "define class Receipt as\n final int number: 7;\n pub get(): int { return self.number; }\nend\npub fnc main(): int { val r = new Receipt(); return r.get(); }";
    assert!(compile(source).is_ok(), "initialized final field should be readable");
    reject(
        "define class Receipt as\n final int number: 7;\n constructor(int n) { self.number = n; }\nend\npub fnc main(): int { val r = new Receipt(9); return 0; }",
        "E_IMMUTABLE",
    );
}

#[test]
fn no_fake_external_or_protected_symbols_appear_in_export_metadata() {
    let source = "external fnc host_clock_ns(): int;\ninternal fnc local(): int { return 42; }\nexport pub fnc answer;\nfnc answer(): int { return local(); }\npub fnc main(): int { return answer(); }";
    let wasm = compile(source).expect("local-only foreign prototype must have no host capability");
    assert!(wasm.windows("atto.exports.v1".len()).any(|w| w == b"atto.exports.v1"));
    assert!(!wasm.windows("host_clock_ns".len()).any(|w| w == b"host_clock_ns"));
    assert!(!wasm.windows("protected".len()).any(|w| w == b"protected"));
}

#[test]
fn internal_functions_preserve_valid_overload_sets() {
    let source = "internal fnc choose(): int { return 1; }\ninternal fnc choose(int n): int { return n; }\npub fnc main(): int { return choose() + choose(41); }\n";
    assert!(compile(source).is_ok());
}

#[test]
fn nested_linkage_modifiers_cannot_escape_their_own_scopes() {
    reject(
        "define class C as\n internal fnc helper(): int { return 42; }\nend\npub fnc main(): int { return 0; }\n",
        "E_SYNTAX",
    );
    reject(
        "define class C as\n external fnc host_clock_ns(): int;\nend\npub fnc main(): int { return 0; }\n",
        "E_SYNTAX",
    );
    reject(
        "pub fnc main(): int { external fnc host_clock_ns(): int; return 0; }",
        "E_SYNTAX",
    );
}
