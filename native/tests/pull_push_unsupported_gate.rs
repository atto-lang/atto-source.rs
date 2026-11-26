//! Explicit absence of actor Flow<T>/Channel<T> support from the scalar-only
//! Rust compiler. Rejection is parity: never silently compile a no-op stream.
use oreslang_source_rs::{compile, frontend};

#[test]
fn pull_push_syntax_fails_closed_in_rust_scalar_frontend() {
    let unsupported = [
        "pub async fnc main(): int { return 1; }",
        "pub fnc main(): int { val Channel<int> ch = Channel.new<int>(1); return 1; }",
        "pub fnc main(): int { return Flow.of(1).next(); }",
        "pub fnc main(): int { nb writech output, 1; return 1; }",
        "pub fnc main(): int { await stream.next(); return 1; }",
        "pub fnc main(): int { return (rt take x); }",
    ];
    for source in unsupported {
        assert!(frontend::parse(source).is_err() || compile(source).is_err(),
            "unsupported demand/actor source unexpectedly compiled: {source}");
    }
    assert!(compile("pub fnc main(): int { return 42; }").is_ok(),
        "negative gate must not reject already-supported scalar programs");
}
