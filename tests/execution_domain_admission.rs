//! Browser compiler admission: exercise the production compile path, not the
//! unlinked historical native frontend (preserved under legacy-native).
use atto_compiler::compile;

fn rejects(source: &str) {
    let err = compile(source).expect_err("unsupported execution domain emitted guest bytes");
    assert!(matches!(err.code, "E_RESERVED" | "E_SYNTAX" | "E_UNSUPPORTED"), "{source}: {err:?}");
}
#[test]
fn domains_are_not_scalar_identifiers_or_execution_regions() {
    for word in ["cpu", "gpu", "quantum"] {
        for source in [
            format!("fnc {word}(): int {{ return 1; }} pub fnc main(): int {{ return 42; }}"),
            format!("fnc f(int {word}): int {{ return {word}; }} pub fnc main(): int {{ return 42; }}"),
            format!("pub fnc main(): int {{ return {word} {{ 1 }}; }}"),
            format!("pub fnc main(): int {{ val int {word}=1; return {word}; }}"),
            format!("pub fnc main(): int {{ return {word} task {{ 1 }}; }}"),
        ] { rejects(&source); }
    }
}
#[test]
fn unqualified_tasks_and_suspension_never_fall_back_to_scalar_execution() {
    for source in [
        "pub fnc main(): int { return task { 1 }; }",
        "pub fnc main(): int { return task scope { 1 }; }",
        "pub fnc main(): int { return nb { 1 }; }",
        "pub fnc main(): int { return await 42; }",
    ] { rejects(source); }
}
#[test]
fn domain_words_in_comments_and_longer_identifiers_are_ordinary() {
    let bytes=compile("// cpu gpu quantum task {}\nfnc compute_gpu(int task_count): int { /* nb { gpu task { 1 } } */ return task_count+1; } pub fnc main(): int { return compute_gpu(41); }").expect("ordinary source compiles");
    assert!(bytes.starts_with(b"\0asm"));
}
