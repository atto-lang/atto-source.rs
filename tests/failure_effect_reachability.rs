//! Reachability-sensitive failure effect regression checks.
//! This is a browser-admitted synchronous subset, not general CFG analysis.
use atto_compiler::compile;

fn wasm(src: &str) {
    let bytes = compile(src).unwrap_or_else(|e| panic!("unexpected effect rejection: {e}\n{src}"));
    assert_eq!(&bytes[..8], b"\0asm\x01\0\0\0");
}

#[test]
fn unreachable_throw_and_calls_do_not_pollute_explicit_bounds() {
    // The declared raises-only upper bound is not violated by an unreachable
    // ordinary throw. The callee's explicit throws(Error) contract is still
    // respected at every reachable call site.
    let prefix = "fnc risky(): int throws(Error) { throw Error(\"boom\"); }\n";
    for body in [
        "return 42; return risky();",
        "return 42; throw Error(\"unreachable\");",
        "if true then return 42; else return 41; fi return risky();",
        "{ return 42; } return risky();",
    ] {
        wasm(&format!("{prefix}pub fnc main(): int raises(Error) {{ {body} }}"));
    }
}

#[test]
fn unreachable_method_calls_do_not_create_false_effect_violations() {
    wasm(r#"
define class Service as
    risky(): int throws(Error) { throw Error("boom"); }
    safe(): int raises(Error) { return 42; return self.risky(); }
end
pub fnc main(): int { val s = new Service(); return s.safe(); }
"#);
}

#[test]
fn reachable_transitive_throws_still_fail_closed() {
    let prefix = "fnc risky(): int throws(Error) { throw Error(\"boom\"); }\n";
    for body in [
        "return risky();",
        "if true then return risky(); fi return 42;",
        // Do not assume an if-branch is unreachable from constant-looking
        // source syntax until a validated CFG establishes that fact.
        "if false then return risky(); fi return 42;",
    ] {
        let source = format!("{prefix}pub fnc main(): int raises(Error) {{ {body} }}");
        let err = compile(&source).expect_err("reachable throw must violate raises-only bound");
        assert_eq!(err.code, "E_EFFECT_TYPE", "{source}: {err}");
    }
}

#[test]
fn explicit_bound_cannot_hide_reachable_method_failure() {
    let source = r#"
define class Service as
    risky(): int throws(Error) { throw Error("boom"); }
    unsafe_call(): int raises(Error) { return self.risky(); }
end
pub fnc main(): int {return 42;}
"#;
    let err = compile(source).expect_err("bad method bound must fail before guest emission");
    assert_eq!(err.code, "E_EFFECT_TYPE", "{err}");
}
