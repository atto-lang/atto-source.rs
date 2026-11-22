//! Fail-closed compiler-core Future-family ownership diagnostics.
//! These tests do NOT pretend ActorFuture async runtime or codegen exists.
use atto_compiler::compile_with_path;
use std::path::Path;
const OWNED: &str = include_str!("../contracts/std-actor-v1/future-owned.accept.atto");
const BORROWED: &str = include_str!("../contracts/std-actor-v1/future-borrowed.reject.atto");
const DOUBLE: &str = include_str!("../contracts/std-actor-v1/future-double-await.reject.atto");
fn code(source: &str) -> &'static str {
    compile_with_path(source, Path::new("stdlib/actor/future_check.atto"))
        .expect_err("no affine Future ABI or privileged runtime is emitted yet").code
}
#[test]
fn affine_actor_future_await_requires_owned_formal_before_any_codegen() {
    assert_eq!(code(BORROWED), "E_OWNERSHIP");
}
#[test]
fn affine_actor_future_double_await_is_use_after_move_before_any_codegen() {
    assert_eq!(code(DOUBLE), "E_MOVED");
}
#[test]
fn valid_owned_actor_future_remains_explicitly_unsupported_until_typed_mir_and_runtime() {
    assert_eq!(code(OWNED), "E_UNSUPPORTED_TYPE");
}
#[test]
fn nested_result_future_does_not_lose_affine_ownership_at_generic_depth() {
    let source = r#"use namespace std.actor;
pub susp fnc sample(take! ActorFuture<Result<int,String>> receipt) -> Result<int,String> {
    return await receipt;
}"#;
    assert_eq!(code(source), "E_UNSUPPORTED_TYPE");
}
#[test]
fn sealed_future_types_cannot_be_guest_constructed_or_user_declared() {
    for source in [
        "use namespace std.actor; define class ActorFuture as end",
        "use namespace std.actor; declare actor TaskFuture;",
        "use namespace std.actor; pub fnc main(): int { val receipt = new Future<int>(); return 42; }",
    ] {
        assert_eq!(code(source), "E_RESERVED");
    }
}
#[test]
fn unrelated_comments_and_strings_do_not_mint_future_capabilities() {
    let source = "// ActorFuture<int> and take! ActorFuture<int> are comments\n\
        use namespace std.actor.local_only;\n\
        pub fnc main(): int { return 42; }";
    let wasm = compile_with_path(source, Path::new("stdlib/actor/local_only.atto"))
        .expect("comments are not Future-family type declarations");
    assert!(wasm.starts_with(b"\0asm\x01\0\0\0"));
}
#[test]
fn borrowed_task_future_is_rejected_before_unsupported_await_error() {
    let source = r#"use namespace std.task;
pub susp fnc sample(TaskFuture<int> receipt) -> int {
    return await receipt;
}"#;
    assert_eq!(code(source), "E_OWNERSHIP");
}
#[test]
fn source_without_future_family_keeps_normal_wasm_compilation() {
    let source = "pub fnc main(): int { return 42; }";
    assert!(compile_with_path(source, Path::new("src/main.atto")).is_ok());
}

#[test]
fn core_future_cannot_be_used_after_successful_await_consumes_receipt() {
    let source=r#"use namespace std.actor;
pub susp fnc sample(take! ActorFuture<int> receipt) -> int {
    val int first = await receipt;
    return receipt;
}"#;
    assert_eq!(code(source),"E_MOVED");
}
#[test]
fn core_future_may_not_be_awaited_while_borrow_is_live() {
    let source=r#"use namespace std.actor;
pub susp fnc sample(take! ActorFuture<int> receipt) -> int {
    val loan = borrow! receipt;
    return await receipt;
}"#;
    assert_eq!(code(source),"E_BORROW");
}
#[test]
fn core_future_take_then_await_is_use_after_move() {
    let source=r#"use namespace std.actor;
pub susp fnc sample(take! ActorFuture<int> receipt) -> int {
    val taken = take! receipt;
    return await receipt;
}"#;
    assert_eq!(code(source),"E_MOVED");
}
