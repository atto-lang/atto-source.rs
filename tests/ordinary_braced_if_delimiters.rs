// Shared ordinary Rust/Wasm compiler delimiter ownership regression.
// Runtime observation is separately exercised by the exact guest carrier.
use atto_compiler::compile;
#[test]
fn braced_statement_does_not_steal_outer_keyword_fi() {
    for source in [
        "pub fnc main(): int { if true { return 42; } else { return 1; } }",
        "pub fnc main(): int { if true do if true { return 42; } else { return 1; } fi return 2; }",
        "pub fnc main(): int { if true { return 42; } else { return 1; } fi }",
    ] {
        assert!(compile(source).is_ok(), "{source}: {:?}", compile(source).err());
    }
    for source in [
        "pub fnc main(): int { if true do return 42; }",
        "pub fnc main(): int { if 1 { return 42; } else { return 1; } }",
        "pub fnc main(): int { if true { return 42; } else { return 1; } fi fi }",
    ] {
        assert!(compile(source).is_err(), "unexpected acceptance: {source}");
    }
}
