//! Real source compiler tests for AOT scalar requires/ensures lowering.
//! Actor supervisor, cleanup/? and typed Error-object semantics remain gated.
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
fn runtime_scalar_predicates_are_emitted_for_both_contract_kinds() {
    emitted("fnc checked(int x) -> int requires x > 0; ensures |r| -> r > 0; :: return x; end fnc main() -> int { return checked(42); }");
    emitted("fnc keep(bool valid) -> bool requires valid; ensures |out| -> out; :: return valid; end fnc main() -> int { if keep(true) then return 42; fi return 0; }");
    emitted("fnc choose(int x) -> int ensures |r| -> r > 0; { if x eq 0 then return 7; fi return 0; } fnc main() -> int { return choose(0); }");
    emitted("fnc good() -> void ensures |ignored| -> true; { return; } fnc main() -> int { good(); return 42; }");
    rejected("define class C as let int count = 0; invariant self.count >= 0; end fnc main() -> int { return 42; }", "E_CONTRACT_UNSUPPORTED");
    rejected("define actor A as invariant |s| -> s.count >= 0; end fnc main() -> int { return 42; }", "E_CONTRACT_UNSUPPORTED");
}

#[test]
fn typed_purity_and_return_capability_fail_closed() {
    rejected("fnc check(int x) -> bool { return x > 0; } fnc guarded(int x) -> int requires check(x); { return x; } fnc main() -> int { return guarded(1); }", "E_CONTRACT_EFFECT");
    rejected("fnc guarded(int x) -> int requires x + 1 > 0; { return x; } fnc main() -> int { return guarded(1); }", "E_CONTRACT_EFFECT");
    rejected("fnc main() -> void ensures |r| -> main() eq true; { return; }", "E_CONTRACT_UNSUPPORTED");
    rejected("fnc checked() -> int ensures |r| -> r; { return 1; } fnc main() -> int { return checked(); }", "E_CONTRACT_NOT_BOOL");
}

#[test]
fn dynamic_contracts_survive_the_parser_as_source_spanned_clauses() {
    let src = "fnc checked(int x) -> int requires x > 0; ensures |result| -> result > 0; { return x; } fnc main() -> int { return checked(1); }";
    let tree = Parser::new(lexer::lex(src).unwrap()).parse_program().unwrap();
    assert_eq!(tree.funcs[0].contracts.len(), 2);
    assert_eq!(tree.funcs[0].contracts[0].pos.line, 1);
}

#[test]
fn misplaced_and_colliding_binders_are_diagnosed() {
    rejected("fnc main(int result) -> int ensures |result| -> true; :: return result; end", "E_CONTRACT_BINDER");
    rejected("fnc main() -> int invariant true; :: return 42; end", "E_CONTRACT_PLACEMENT");
    rejected("fnc main() -> int ensures |result| -> true; requires true; :: return 42; end", "E_CONTRACT_ORDER");
    rejected("fnc main() -> int ensures |self| -> true; :: return 42; end", "E_CONTRACT_BINDER");
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
