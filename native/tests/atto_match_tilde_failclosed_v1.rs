//! Safety parity for canonical match and tilde grammar while the Rust/Cranelift
//! stage-0 compiler remains a checked scalar-only frontend.
//! This test does NOT certify match execution, typed Map ownership or AOT parity.
use oreslang_source_rs::{compile, frontend, mir};

#[test]
fn rejected_match_and_tilde_do_not_emit_a_native_object_or_mir() {
    const UNSUPPORTED: [&str; 10] = [
        "pub fnc main(): int { return match<int> 9 over on 1 -> { 11; } default -> { 42; } end; }",
        "pub fnc main(): int { return match 9 over default -> { 42; } end; }",
        "pub fnc main(): int { return match 9 over on default -> { 42; } end; }",
        "fnc never(): int { return match 1 over default -> { 42; } end; } pub fnc main(): int { return 42; }",
        "pub fnc main(): int { const x = ~map<string,int>(\"answer\" => 42); return 42; }",
        "pub fnc main(): int { return ~record(foo: 42); }",
        "pub fnc main(): int { return ~(foo: 42); }",
        "pub fnc main(): int { return ~(); }",
        "pub fnc main(): int { const x = ~map(); return 42; }",
        "pub fnc main(): int { const x = ~map<Symbol,bool>(#ready => true); return 42; }",
    ];
    for input in UNSUPPORTED {
        assert!(frontend::parse(input).is_err(), "parser admitted unsupported: {input}");
        assert!(compile(input).is_err(), "native object published for unsupported: {input}");
    }
}

#[test]
fn actual_compiler_baseline_remains_checked_scalar_and_comments_are_invisible() {
    let input = "pub fnc main(): int { /* match<int> x over ~map() */ // default -> {}\n return 40 + 2; }";
    let parsed = frontend::parse(input).expect("comments do not become language tokens");
    mir::lower(parsed).expect("baseline scalar MIR is still valid");
    let obj = compile(input).expect("baseline scalar should compile to an object");
    assert!(!obj.is_empty());
    #[cfg(target_os = "linux")]
    assert!(obj.starts_with(b"\x7fELF"), "native compiler must emit real ELF");
}
