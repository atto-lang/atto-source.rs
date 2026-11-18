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
fn canonical_defer_spelling_factories_and_recover_error_views_compile() {
    pass(r#"
        fnc failing(): int raises(Error) { raise Error("boom"); }
        fnc f(int seed): int {
            defer { stdio.println("block"); };
            defer _ -> { stdio.println("underscore"); };
            defer || -> { stdio.println("legacy"); };
            defer (|int value| -> _ -> { stdio.println(value); })(seed);
            recover |err, cursor| -> {
                stdio.println(err.message);
                stdio.println(err.is_raise);
                return 6;
            };
            recover |ErrorView reason, RecoveryCursor next| -> {
                stdio.println(reason.message);
                next.next();
            };
            return failing();
        }
        pub fnc main(): int { return f(7); }
    "#);
}

#[test]
fn canonical_cleanup_rejects_bad_factories_and_escaping_handles() {
    for (code, s) in [
        ("E_ARITY", r#"pub fnc main(): int {
            defer (|int v| -> _ -> { stdio.println(v); })();
            return 0; }"#),
        ("E_TYPE", r#"pub fnc main(): int {
            defer (|int v| -> _ -> { stdio.println(v); })("wrong");
            return 0; }"#),
        ("E_DUPLICATE", r#"pub fnc main(): int {
            defer (|int v, int v| -> _ -> { stdio.println(v); })(1, 2);
            return 0; }"#),
        ("E_UNBOUND", r#"pub fnc main(): int {
            val int n = 4;
            defer _ -> { stdio.println(n); };
            return 0; }"#),
        ("E_UNBOUND", r#"pub fnc main(): int {
            recover |err, c| -> { return err; };
            raise Error("fail"); }"#),
        ("E_UNBOUND", r#"pub fnc main(): int {
            recover |err, c| -> { return c; };
            raise Error("fail"); }"#),
        ("E_UNBOUND", r#"pub fnc main(): int {
            recover |err, c| -> { stdio.println(err.unknown); return 2; };
            raise Error("fail"); }"#),
        ("E_DUPLICATE", r#"pub fnc main(): int {
            recover |reason, reason| -> { return 2; };
            raise Error("fail"); }"#),
        ("E_RETURN", r#"pub fnc main(): int {
            recover |reason, c| -> { stdio.println(reason.message); };
            raise Error("fail"); }"#),
    ] {
        reject(code, s);
    }
}

#[test]
fn cleanup_call_graph_must_prove_no_transitive_exception() {
    // No user-defined call may replace a pending failure from cleanup,
    // including hidden calls in expression arguments, try/finally and
    // recovery handlers. The fixed point checks chains of arbitrary length.
    for source in [
        r#"fnc bad(): int { throw Error("throw"); }
            pub fnc main(): int {
                defer { stdio.println(bad()); };
                return 2;
            }"#,
        r#"fnc bad(): int { raise Error("raise"); }
            fnc middle(): int { return bad(); }
            fnc outer(): int { return middle(); }
            pub fnc main(): int {
                defer (|int v| -> _ -> { stdio.println(outer()); })(5);
                return 2;
            }"#,
        r#"fnc bad(): int { raise Error("raise"); }
            pub fnc main(): int {
                recover |err, cursor| -> {
                    stdio.println(bad());
                    return 10;
                };
                raise Error("original");
            }"#,
        r#"fnc bad(): int { throw Error("throw"); }
            pub fnc main(): int {
                try { return 3; }
                finally { stdio.println(bad()); }
            }"#,
    ] {
        reject("E_UNWIND_EFFECT", source);
    }
    // A pure/safe callable is not barred merely because it is user-defined.
    // Side-effecting nonthrowing helpers may still run as deferred cleanup.
    pass(r#"
        fnc safe(int n): int { return n + 1; }
        fnc print_safely(int n): void { stdio.println(safe(n)); }
        pub fnc main(): int {
            defer (|int value| -> _ -> { print_safely(value); })(5);
            recover |err, cursor| -> { return safe(10); };
            return 4;
        }
    "#);
}

#[test]
fn handled_exception_effects_are_accepted_in_cleanup() {
    // The previous transitive-any-throw scanner rejected all of these,
    // even though ordinary throws do not escape their typed handler.
    for source in [
        r#"fnc fail(): int { throw Error("caught"); }
           fnc safe(): int {
             try { return fail(); }
             catch (Error e) { return 11; }
           }
           pub fnc main(): int {
             defer { stdio.println(safe()); };
             return 22;
           }"#,
        r#"fnc fail(): int { throw Error("caught"); }
           fnc safe(): int {
             val Option<int> caught = trap fail();
             return 17;
           }
           pub fnc main(): int {
             defer { stdio.println(safe()); };
             return 22;
           }"#,
        r#"fnc fail(): int { throw Error("caught"); }
           fnc safe(): int {
             try { return fail(); }
             catch (Error e) { return 11; }
           }
           fnc middle(): int { return safe(); }
           pub fnc main(): int {
             recover |err, cursor| -> { return middle(); };
             raise Error("trigger");
           }"#,
        r#"fnc fail(): int { throw Error("caught"); }
           fnc safe(): int {
             try { return fail(); }
             catch (Error e) { return 11; }
           }
           pub fnc main(): int {
             try { return 22; }
             finally { stdio.println(safe()); }
           }"#,
        r#"fnc fail(): int { throw Error("caught"); }
           fnc safe(): int {
             try { return fail(); }
             catch (Error e) {
               try { return 11; }
               finally { stdio.println("inner"); }
             }
           }
           pub fnc main(): int {
             defer { stdio.println(safe()); };
             return 22;
           }"#,
        r#"fnc fail(): int { throw Error("caught"); }
           fnc safe(): int {
             try { return fail(); }
             catch (Error e) { return 11; }
           }
           fnc a(): int { return b(); }
           fnc b(): int { return safe(); }
           pub fnc main(): int {
             defer { stdio.println(a()); };
             return 22;
           }"#,
        r#"fnc fail(): int { throw Error("caught"); }
           fnc a(int depth): int {
             if depth eq 0 then return 1; fi
             try { return b(depth - 1); }
             catch (Error e) { return 8; }
           }
           fnc b(int depth): int { return a(depth); }
           pub fnc main(): int {
             defer { stdio.println(a(2)); };
             return 22;
           }"#,
    ] {
        pass(source);
    }
}

#[test]
fn escaping_effects_remain_forbidden_in_cleanup() {
    // Ordinary catch is insufficient for raise, and trap never catches it.
    // The catch handler's *own* failure must also count as escaping.
    for source in [
        r#"fnc fail(): int { raise Error("uncaught raise"); }
           fnc unsafe_catch(): int {
             try { return fail(); }
             catch (Error e) { return 11; }
           }
           pub fnc main(): int {
             defer { stdio.println(unsafe_catch()); };
             return 2;
           }"#,
        r#"fnc fail(): int { raise Error("uncaught raise"); }
           fnc unsafe_trap(): int {
             val Option<int> caught = trap fail();
             return 11;
           }
           pub fnc main(): int {
             recover |err, cursor| -> { return unsafe_trap(); };
             raise Error("original");
           }"#,
        r#"fnc fail(): int { throw Error("caught first"); }
           fnc unsafe_handler(): int {
             try { return fail(); }
             catch (Error e) { raise Error("raised by handler"); }
           }
           pub fnc main(): int {
             defer { stdio.println(unsafe_handler()); };
             return 2;
           }"#,
        r#"fnc fail(): int { throw Error("caught first"); }
           fnc unsafe_handler(): int {
             try { return fail(); }
             catch (Error e) { throw Error("thrown by handler"); }
           }
           pub fnc main(): int {
             try { return 2; }
             finally { stdio.println(unsafe_handler()); }
           }"#,
        r#"fnc fail(): int { raise Error("raised"); }
           fnc middle(): int { return fail(); }
           fnc unsafe_catch(): int {
             try { return middle(); }
             catch (Error e) { return 11; }
           }
           pub fnc main(): int {
             defer { stdio.println(unsafe_catch()); };
             return 2;
           }"#,
        r#"fnc fail(): int { throw Error("thrown"); }
           fnc unsafe_catch(): int {
             try { return fail(); }
             finally { stdio.println("not a catch"); }
           }
           pub fnc main(): int {
             defer { stdio.println(unsafe_catch()); };
             return 2;
           }"#,
        r#"fnc fail(): int { raise Error("raised"); }
           fnc a(int depth): int {
             if depth eq 0 then return fail(); fi
             return b(depth - 1);
           }
           fnc b(int depth): int { return a(depth); }
           pub fnc main(): int {
             defer { stdio.println(a(1)); };
             return 2;
           }"#,
    ] {
        reject("E_UNWIND_EFFECT", source);
    }
}

#[test]
fn effect_solver_exhaustive_cleanup_matrix() {
    // 3 cleanup contexts × 2 handler forms × 10 distinct argument values:
    // sixty real, deterministic compiled guest artifacts in addition to
    // the individually named positive/negative effect regressions above.
    for n in 0..10 {
        for uses_trap in [false, true] {
            let safe = if uses_trap {
                r#"val Option<int> captured = trap fail(); return 8;"#
            } else {
                r#"try { return fail(); } catch (Error e) { return 8; }"#
            };
            let preamble = format!(
                r#"fnc fail(): int {{ throw Error("handled"); }}
                   fnc safe(): int {{ {safe} }}
                   fnc intermediate(): int {{ return safe(); }}"#,
            );
            let callbacks = [
                format!(r#"defer (|int value| -> _ -> {{ stdio.println(intermediate() + value); }})({n});
                            return 2;"#),
                format!(r#"recover (|int value| -> |err, cursor| -> {{
                              return intermediate() + value;
                            }})({n});
                            raise Error("trigger");"#),
                format!(r#"try {{ return 2; }}
                            finally {{ stdio.println(intermediate() + {n}); }}"#),
            ];
            for body in callbacks {
                pass(&format!("{preamble} pub fnc main(): int {{ {body} }}"));
            }
        }
    }
}

#[test]
fn recovery_capability_binders_cannot_be_shadowed_in_any_scope() {
    for source in [
        r#"pub fnc main(): int {
            recover |err, cursor| -> {
                let int err = 7;
                return 9;
            };
            raise Error("x");
        }"#,
        r#"pub fnc main(): int {
            recover |err, cursor| -> {
                let int cursor = 1;
                return 9;
            };
            raise Error("x");
        }"#,
        r#"pub fnc main(): int {
            recover |err, cursor| -> {
                if true then
                    val int err = 3;
                fi
                return 9;
            };
            raise Error("x");
        }"#,
    ] {
        reject("E_RESERVED", source);
    }
}

#[test]
fn programs_without_unwind_constructs_keep_their_guest_bytes() {
    // The unwind lowering is opt-in: no extra tag, global or frame for
    // existing throw/trap programs.
    let plain = r#"
        fnc t(): int { throw Error("x"); }
        pub fnc main(): int {
          val Option<int> r = trap t();
          return 0;
        }
    "#;
    let bytes = compile(plain).unwrap();
    // exactly one tag (section 13: id, size, count=1)
    let tag = bytes.windows(3).position(|w| w == [13, 3, 1]);
    assert!(tag.is_some(), "single private ordinary-throw tag expected");
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
