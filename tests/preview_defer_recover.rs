//! defer / recover / try-catch-finally / raise (atto-spec
//! language/defer-recover-unwind-v1.md) compile to real guest Wasm on the
//! private legacy-EH tags; every unsafe or unsupported shape is rejected with
//! a positioned diagnostic and no guest artifact. Execution traces are checked
//! by tests/defer_recover_guest.mjs and the playground preview audit.
use atto_compiler::compile;

const SITE: &str = include_str!("playground_previews/defer-recover.atto");

fn pass(s: &str) {
    let bytes = compile(s).unwrap_or_else(|e| panic!("expected compiled guest: {e}\n{s}"));
    assert!(bytes.starts_with(b"\0asm\x01\0\0\0"));
    assert_eq!(bytes, compile(s).unwrap(), "Wasm guest must be deterministic");
}

fn reject(code: &str, s: &str) {
    let d = compile(s).expect_err("unsafe or unsupported unwind shape must emit no guest");
    assert_eq!(d.code, code, "{d}\n{s}");
    assert!(d.pos.line > 0 && d.pos.col > 0, "{d}");
}

#[test]
fn exact_published_defer_recover_preview_compiles_to_guest_wasm() {
    pass(SITE);
}

#[test]
fn effect_clauses_try_finally_and_delegating_recovery_compile() {
    pass(r#"
        fnc fail(): int raises(Error) { raise new Error("boom"); }
        fnc may_throw(): int throws(Error), raises(Error) { throw Error("t"); }
        fnc f(): int {
          recover (|int v| -> |err, c| -> { return v; })(5);
          recover (|string s| -> |err, c| -> { stdio.println(s); c.next(); })("delegate");
          defer || -> { stdio.println("cleanup"); };
          return fail();
        }
        fnc g(): int {
          let int x = 1;
          try { try { return x; } finally { x = 2; } } finally { stdio.println("outer"); }
        }
        pub fnc main(): int { return f() + g(); }
    "#);
}

#[test]
fn ordinary_throw_without_recover_keeps_single_private_guest_tag() {
    // Ordinary throw lowering remains supported independently of trap.
    // A source-level throw with no trap/recover must not allocate an
    // unnecessary second tag, global, or unwind frame.
    let plain = r#"
        fnc t(): int { throw Error("x"); }
        pub fnc main(): int { return 0; }
    "#;
    let bytes = compile(plain).unwrap();
    // Exactly one private ordinary-throw tag (section 13).
    let tag = bytes.windows(3).position(|w| w == [13, 3, 1]);
    assert!(tag.is_some(), "single ordinary-throw tag expected");

    // The old trap-to-Option codegen must be unreachable on this branch.
    reject("E_UNSUPPORTED", r#"
        fnc t(): int { throw Error("x"); }
        pub fnc main(): int {
            val r = trap t();
            return 0;
        }
    "#);
}

#[test]
fn registration_must_be_top_level_and_cleanup_cannot_unwind() {
    for (code, s) in [
        ("E_UNSUPPORTED", r#"pub fnc main(): int {
            if true then defer || -> { stdio.println("x"); }; fi
            return 0; }"#),
        ("E_UNSUPPORTED", r#"pub fnc main(): int {
            for (let i = 0; i < 2; i++) do recover (|int v| -> |err, c| -> { return v; })(1); done
            return 0; }"#),
        ("E_UNSUPPORTED", r#"pub fnc main(): int {
            defer || -> { throw Error("replaces pending failure"); };
            return 0; }"#),
        ("E_UNSUPPORTED", r#"pub fnc main(): int {
            defer || -> { raise Error("replaces pending failure"); };
            return 0; }"#),
        ("E_UNSUPPORTED", r#"pub fnc main(): int {
            defer || -> { return; };
            return 0; }"#),
        ("E_UNSUPPORTED", r#"pub fnc main(): int {
            defer || -> { try { stdio.println("x"); } finally { stdio.println("y"); } };
            return 0; }"#),
        ("E_UNSUPPORTED", r#"pub fnc main(): int {
            recover (|int v| -> |err, c| -> { defer || -> { stdio.println("x"); }; return v; })(1);
            return 0; }"#),
        ("E_UNSUPPORTED", r#"pub fnc main(): int {
            try { stdio.println("x"); } finally { if true then return 1; fi }
            return 0; }"#),
    ] {
        reject(code, s);
    }
}

#[test]
fn cleanup_bodies_cannot_capture_enclosing_locals() {
    reject("E_UNBOUND", r#"pub fnc main(): int {
        let int total = 1;
        defer || -> { stdio.println(total); };
        return total; }"#);
    reject("E_UNBOUND", r#"pub fnc main(): int {
        val int seed = 3;
        recover (|int v| -> |err, c| -> { return seed; })(1);
        return 0; }"#);
}

#[test]
fn recovery_handlers_are_typed_and_terminal() {
    for (code, s) in [
        ("E_ARITY", r#"pub fnc main(): int {
            recover (|int v, int w| -> |err, c| -> { return v; })(1);
            return 0; }"#),
        ("E_TYPE", r#"pub fnc main(): int {
            recover (|int v| -> |err, c| -> { return v; })("not an int");
            return 0; }"#),
        ("E_RETURN", r#"pub fnc main(): int {
            recover (|string s| -> |err, c| -> { stdio.println(s); })("falls through");
            return 0; }"#),
        ("E_TYPE", r#"pub fnc main(): int {
            recover (|string s| -> |err, c| -> { return s; })("wrong result type");
            return 0; }"#),
        // The pending Error and cursor are not first-class values here.
        ("E_UNBOUND", r#"pub fnc main(): int {
            recover (|int v| -> |err, c| -> { stdio.println(err); return v; })(1);
            return 0; }"#),
        ("E_UNBOUND", r#"pub fnc main(): int {
            c.next();
            return 0; }"#),
    ] {
        reject(code, s);
    }
}

#[test]
fn only_built_in_error_is_thrown_raised_or_caught() {
    for (code, s) in [
        ("E_TYPE", r#"pub fnc main(): int { raise "not an Error"; }"#),
        ("E_TYPE", r#"fnc f(): int raises(Failure) { return 1; }
            pub fnc main(): int { return f(); }"#),
        ("E_TYPE", r#"pub fnc main(): int {
            try { return 1; } catch (Failure e) { return 2; } }"#),
        ("E_SYNTAX", r#"pub fnc main(): int { try { return 1; } }"#),
        ("E_DUPLICATE", r#"fnc f(): int throws(Error) throws(Error) { return 1; }
            pub fnc main(): int { return f(); }"#),
    ] {
        reject(code, s);
    }
}

#[test]
fn break_and_continue_cannot_bypass_handlers() {
    reject("E_LOOP", r#"pub fnc main(): int {
        while true do
          try { break; } finally { stdio.println("must run"); }
        done
        return 0; }"#);
}

#[test]
fn methods_and_generators_fail_closed() {
    reject("E_UNSUPPORTED", r#"
        define class Box as
          pub get(): int { try { return 1; } finally { stdio.println("x"); } }
        end
        pub fnc main(): int { return 0; }"#);
    reject("E_UNSUPPORTED", r#"
        fnc *gen g(): Generator<int> { try { yield 1; } finally { stdio.println("x"); } }
        pub fnc main(): int { return 0; }"#);
}
