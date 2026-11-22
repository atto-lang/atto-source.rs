//! Real source compiler tests for the deliberately bounded compile-time-proved
//! contract subset. A passing test does not certify dynamic contracts, actor
//! invariant runtime checks, or the typed ContractViolationError ABI.
use atto_compiler::{compile, lexer, parser::Parser};

fn emitted(source: &str) {
    let wasm = compile(source).unwrap_or_else(|e| panic!("expected compiled guest: {e}"));
    assert!(wasm.starts_with(b"\0asm"), "must emit actual Wasm, not an empty stub");
}

fn rejected(source: &str, code: &str) {
    let error = compile(source).expect_err("compiler MUST NOT emit a guest artifact");
    assert_eq!(error.code, code, "unexpected diagnostic: {error}");
}

#[test]
fn proven_true_requires_and_ensures_keep_full_guest_aot() {
    emitted("fnc main() -> int requires true; ensures |result| -> true; :: return 42; end");
    emitted("fnc main() -> int requires !false; ensures |result| -> !false; :: return 42; end");
    emitted("fnc int <- main() requires true; return 42; end");
    emitted("fnc main() : int requires true; { return 42; }");
}

#[test]
fn proven_true_method_and_class_invariant_parse_without_runtime_magic() {
    let src = "define class Crate as invariant true; get() -> int requires true; ensures |r| -> true; { return 42; } end fnc main() -> int { return 42; }";
    let tree = Parser::new(lexer::lex(src).unwrap()).parse_program().unwrap();
    assert_eq!(tree.classes.len(), 1);
    assert_eq!(tree.classes[0].methods.len(), 1);
}

#[test]
fn proven_true_actor_invariant_is_parsed_but_not_an_actor_runtime_proof() {
    let src = "define actor Worker as invariant |s| -> true; end fnc main() -> int { return 42; }";
    let tree = Parser::new(lexer::lex(src).unwrap()).parse_program().unwrap();
    assert_eq!(tree.actors.len(), 1);
}

#[test]
fn constant_false_never_becomes_a_runtime_assumption() {
    rejected("fnc main() -> int requires false; :: return 42; end", "E_CONTRACT_ALWAYS_FALSE");
    rejected("fnc main() -> int ensures |r| -> !true; :: return 42; end", "E_CONTRACT_ALWAYS_FALSE");
    rejected("define class C as invariant false; end fnc main() -> int { return 42; }", "E_CONTRACT_ALWAYS_FALSE");
}

#[test]
fn non_boolean_contracts_reject_without_artifacts() {
    rejected("fnc main() -> int requires 1; :: return 42; end", "E_CONTRACT_NOT_BOOL");
    rejected("fnc main(int value) -> int requires value; :: return value; end", "E_CONTRACT_NOT_BOOL");
    rejected("fnc main() -> int ensures |r| -> r; :: return 42; end", "E_CONTRACT_NOT_BOOL");
}

#[test]
fn dynamic_predicates_never_silently_disappear_from_release_wasm() {
    rejected("fnc main(int value) -> int requires value > 0; :: return value; end", "E_CONTRACT_UNSUPPORTED");
    rejected("fnc main() -> int ensures |r| -> r > 0; :: return 42; end", "E_CONTRACT_UNSUPPORTED");
    rejected("define class C as let int count = 0; invariant self.count >= 0; end fnc main() -> int { return 42; }", "E_CONTRACT_UNSUPPORTED");
    rejected("define actor A as invariant |s| -> s.count >= 0; end fnc main() -> int { return 42; }", "E_CONTRACT_UNSUPPORTED");
}

#[test]
fn misplaced_and_colliding_binders_are_diagnosed() {
    rejected("fnc main(int result) -> int ensures |result| -> true; :: return result; end", "E_CONTRACT_BINDER");
    rejected("fnc main() -> int invariant true; :: return 42; end", "E_CONTRACT_PLACEMENT");
    rejected("define class C as invariant |s| -> true; end fnc main() -> int { return 42; }", "E_CONTRACT_PLACEMENT");
}

#[test]
fn malformed_headers_are_not_accepted() {
    for src in [
        "fnc main() -> int requires ; :: return 42; end",
        "fnc main() -> int ensures |r -> true; :: return 42; end",
        "fnc main() -> int ensures || -> true; :: return 42; end",
        "fnc main() -> int requires true ensures |r| -> true; :: return 42; end",
    ] {
        assert!(compile(src).is_err(), "malformed contract was compiled: {src}");
    }
}

#[test]
fn contract_keywords_cannot_be_used_to_bypass_the_header_admission_gate() {
    for src in [
        "fnc main() -> int { requires true; return 42; }",
        "fnc main() -> int { ensures |result| -> true; return 42; }",
    ] {
        assert!(compile(src).is_err(), "body-local contract bypass: {src}");
    }
}
