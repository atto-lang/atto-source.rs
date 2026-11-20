//! Restricted source-compiled List<int> match tests.
//! A real guest run additionally requires the Node Wasm execution harness.
use atto_compiler::{compile,lexer,list_pattern_guest};
const BASE: &str=include_str!("fixtures/pass/list-pattern-rest-60.atto");
fn rejection(source:&str,expected:&str){
    let err=compile(source).expect_err("unsupported source MUST NOT emit wasm");
    assert_eq!(err.code,expected,"{source}: {err:?}");
}
#[test]
fn checked_heap_ir_is_produced_by_actual_compiler(){
    let tokens=lexer::lex(BASE).unwrap();
    let program=list_pattern_guest::try_lower(&tokens).unwrap().unwrap();
    assert!(program.heap);
    assert_eq!(program.funcs.len(),1);
    assert_eq!(program.funcs[0].locals.len(),1);
    let bytecode=compile(BASE).expect("a checked narrow List source compiles to actual Wasm");
    assert!(bytecode.len()>40);
    assert!(bytecode.starts_with(b"\0asm"));
}
#[test]
fn other_compiled_guests_still_emit_wasm(){
    for src in [
        include_str!("fixtures/pass/list-pattern-guard-once-0.atto"),
        include_str!("fixtures/pass/list-pattern-infix-30.atto"),
        include_str!("fixtures/pass/list-pattern-anchored-no-match-7.atto"),
        include_str!("fixtures/pass/list-pattern-full-width-1.atto"),
        include_str!("fixtures/pass/list-pattern-leftmost-3.atto"),
        include_str!("fixtures/pass/list-pattern-max-length-32.atto"),
        include_str!("fixtures/pass/list-pattern-standalone-rest-11.atto"),
        include_str!("fixtures/pass/list-pattern-empty-list-9.atto"),
        include_str!("fixtures/pass/list-pattern-empty-rest-13.atto"),
    ] {
        let result=compile(src).expect("bounded guest match source");
        assert!(result.starts_with(b"\0asm"));
    }
}
#[test]
fn rejects_other_types_and_nonexistent_bindings(){
    rejection(&BASE.replace("is int as val b","is bool as val b"),"E_UNSUPPORTED");
    rejection(&BASE.replace("is int as val b","is int as val a"),"E_PATTERN_BINDING");
    rejection(&BASE.replace("a + b + c","a + missing + c"),"E_PATTERN_BINDING");
    // Captures are arm-local; the independent wildcard fallback cannot use a.
    rejection(&BASE.replace("on _ -> 0", "on _ -> a"),"E_PATTERN_BINDING");
    let guarded=include_str!("fixtures/pass/list-pattern-guard-once-0.atto");
    rejection(&guarded.replace("when a > 0","when missing > 0"),"E_PATTERN_BINDING");
    rejection(&BASE.replace("on List.of(...,","on List.of(..., ...,"),
        "E_PATTERN_SOURCE");
}
#[test]
fn rejects_bad_constructor_and_unverified_patterns(){
    rejection(&BASE.replace("List.of(10, 20, 30, 40, 50)",
        "List.of(10, 20,)"),"E_PATTERN_SOURCE");
    rejection(&BASE.replace("..., is int as val a",
        "...(2 of int), is int as val a"),"E_UNSUPPORTED");
    rejection(&BASE.replace("on List.of(..., is int as val a",
        "on List.of(..., pattern.of<int>(\"...\"), is int as val a"),"E_UNSUPPORTED");
    rejection(&BASE.replace("val items: List<int>","let items: List<int>"),
        "E_UNSUPPORTED");
}
#[test]
fn oversize_sequences_are_rejected_before_artifact(){
    let source=BASE.replace("10, 20, 30, 40, 50",
        &(0..33).map(|v|v.to_string()).collect::<Vec<_>>().join(", "));
    rejection(&source,"E_PATTERN_BUDGET");
}
#[test]
fn scalar_match_compiler_does_not_acquire_list_privileges(){
    let program="pub fnc main(): int { return match 1 over on 1 -> 42, default -> 0, end }";
    let out=compile(program).expect("existing scalar match must remain valid");
    assert!(out.starts_with(b"\0asm"));
}

#[test]
fn signed_minimum_and_empty_list_shape_are_exact() {
    let minimum=BASE.replace("10, 20, 30, 40, 50",
        "-9223372036854775808, 20, 30, 40, 50");
    assert!(compile(&minimum).unwrap().starts_with(b"\0asm"));
    rejection(&BASE.replace("10, 20, 30, 40, 50",
        "9223372036854775808, 20, 30, 40, 50"),"E_INTEGER_RANGE");
    let empty=BASE.replace("10, 20, 30, 40, 50","")
        .replace("..., is int as val a, is int as val b, is int as val c, ...",
            "...");
    rejection(&empty,"E_PATTERN_BINDING");
}

#[test]
fn fixed_atom_budget_applies_after_removing_rest_wildcards() {
    const OLD: &str =
        "on List.of(..., is int as val a, is int as val b, is int as val c, ...) -> a + b + c,";
    assert!(BASE.contains(OLD), "budget test must rewrite the actual fixture");

    // Exactly 16 fixed atoms are legal even when the shorter subject
    // makes the first arm impossible; falling to default remains safe.
    let max_valid = (0..16).map(|_| "is int").collect::<Vec<_>>().join(", ");
    let valid = BASE.replace(OLD, &format!("on List.of({max_valid}) -> 1,"));
    assert!(compile(&valid).is_ok(), "boundary at fixed atom budget");

    // There are no rests here, so a token limit of MAX_FIXED + 2 would
    // accidentally admit 17/18 fixed atoms. The typed IR limit is 16.
    for count in [17, 18, 19] {
        let parts = (0..count).map(|_| "is int").collect::<Vec<_>>().join(", ");
        let invalid = BASE.replace(OLD, &format!("on List.of({parts}) -> 1,"));
        rejection(&invalid, "E_PATTERN_BUDGET");
    }
}

#[test]
fn reserved_names_and_adjacent_rests_fail_before_guest_artifact() {
    let bad_subject = BASE.replace("val items: List<int>", "val match: List<int>")
        .replace("match<int> items over", "match<int> match over");
    rejection(&bad_subject, "E_RESERVED");

    let bad_binder = BASE.replace("is int as val a", "is int as val return")
        .replace("a + b + c", "return + b + c");
    rejection(&bad_binder, "E_RESERVED");

    // Adjacent rests are malformed even if they occur after a fixed atom.
    for pat in ["..., ..., is int as val a",
                "is int as val a, ..., ..."] {
        let source = BASE.replace("..., is int as val a", pat);
        rejection(&source, "E_PATTERN_SOURCE");
    }

    // Interior nonadjacent rests are valid in the broader specification,
    // but are deliberately not executable under this restricted profile.
    let interior = BASE.replace("is int as val b, is int as val c",
        "is int as val b, ..., is int as val c");
    rejection(&interior, "E_UNSUPPORTED");
}

#[test]
fn empty_list_and_standalone_rest_use_checked_guest_representation() {
    let empty = r#"
        pub fnc main(): int {
          val items: List<int> = List.of();
          return match<int> items over
            on List.of() -> 9,
            on _ -> 0
          end;
        }
    "#;
    let empty_program = list_pattern_guest::try_lower(
        &lexer::lex(empty).expect("valid tokens")).unwrap().unwrap();
    assert!(empty_program.heap);
    assert!(compile(empty).unwrap().starts_with(b"\0asm"));

    let any_length = BASE.replace(
        "on List.of(..., is int as val a, is int as val b, is int as val c, ...) -> a + b + c,",
        "on List.of(...) -> 1,",
    );
    assert_ne!(any_length, BASE, "test must replace actual pattern");
    assert!(compile(&any_length).unwrap().starts_with(b"\0asm"));
}

#[test]
fn lone_rest_matches_nonempty_and_empty_without_branch_budget_exhaustion() {
    for source in [
        include_str!("fixtures/pass/list-pattern-standalone-rest-11.atto"),
        include_str!("fixtures/pass/list-pattern-empty-rest-13.atto"),
    ] {
        let checked = list_pattern_guest::try_lower(&lexer::lex(source).unwrap())
            .unwrap().expect("single rest should be accepted in checked IR");
        assert!(checked.heap);
        assert_eq!(checked.funcs.len(), 1);
        assert!(compile(source).unwrap().starts_with(b"\\0asm"));
    }
}
