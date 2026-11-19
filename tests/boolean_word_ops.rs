// Named boolean operators should be as executable as &&/||, with the
// same short-circuit and type-checking behavior on the browser Rust backend.
use atto_compiler::compile;

#[test]
fn word_and_or_compile_in_function_returns_and_conditions() {
    let programs = [
        "fnc both(bool a, bool b): bool { return a and b; } pub fnc main(): int { if both(true, true) then return 42; else return 0; fi }",
        "fnc either(bool a, bool b): bool { return a or b; } pub fnc main(): int { if either(false, true) then return 42; else return 0; fi }",
        "pub fnc main(): int { if true or false and false then return 42; else return 0; fi }",
        "pub fnc main(): int { if false or true and true then return 42; else return 0; fi }",
        "pub fnc main(): int { if true && true or false then return 42; else return 0; fi }",
    ];
    for p in programs {
        assert!(compile(p).is_ok(), "word boolean operator rejected: {p}: {:?}", compile(p).err());
    }
}

#[test]
fn word_operators_reject_untyped_non_bool_operands() {
    let bad = [
        "pub fnc main(): int { if 42 and true then return 1; else return 0; fi }",
        "pub fnc main(): int { if false or 3 then return 1; else return 0; fi }",
        "pub fnc main(): int { return 1 or 2; }",
    ];
    for p in bad {
        assert!(compile(p).is_err(), "unexpectedly allowed non-bool: {p}");
    }
}
