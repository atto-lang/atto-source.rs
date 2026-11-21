//! Real compiler API acceptance/rejection tests for closed-world Mutex<int>.
 //! Guest execution is verified separately by tests/mutex_fast_guest_execution.mjs
 //! using a compiled native attoc; these Rust checks do not execute Wasm.
use atto_compiler::compile;

const INCREMENT: &str = r#"
susp fnc increment(Mutex<int> counter): int {
    using await counter.lock_async() as lease {
        lease.value += 1;
        return lease.value;
    }
}
"#;
const MAIN: &str = r#"
pub susp fnc main(): int {
    val counter = new Mutex<int>(0);
    return await increment(counter);
}
"#;

fn source(main: &str) -> String { format!("{INCREMENT}{main}") }

#[test]
fn compiles_checked_scalar_fast_path_into_deterministic_wasm() {
    let s = source(MAIN);
    let a = compile(&s).expect("verified single-task profile");
    let b = compile(&s).expect("same source must be deterministic");
    assert!(a.starts_with(b"\0asm"));
    assert_eq!(a,b);
}

#[test]
fn scalar_update_is_compiled_not_replaced_by_fixed_result() {
    for (initial,delta) in [(0,1), (17,23),(-1,1),(-20,3),(20,-3)] {
        let source = source(MAIN)
            .replace("new Mutex<int>(0)", &format!("new Mutex<int>({initial})"))
            .replace("lease.value += 1;", &format!("lease.value += {delta};"));
        let wasm=compile(&source).expect("checked runtime scalar expression");
        assert!(wasm.starts_with(b"\0asm"), "{initial}+{delta}");
    }
}

#[test]
fn compiler_accepts_implicit_suspension_alias_without_changing_wasm() {
    let s=source(MAIN);
    assert_eq!(compile(&s).unwrap(),
        compile(&s.replace("using await counter.lock_async()", "using counter.lock_async()")).unwrap());
    assert_eq!(compile(&s).unwrap(),
        compile(&s.replace("susp fnc", "suspends fnc")).unwrap());
}

#[test]
fn lazy_unawaited_task_has_no_execution_path_to_increment() {
    let lazy=r#"
pub fnc main(): int {
    val counter = new Mutex<int>(0);
    val pending = increment(counter);
    return 0;
}
"#;
    let out=compile(&source(lazy)).expect("unstarted lazy task is not run");
    assert!(out.starts_with(b"\0asm"));
}

#[test]
fn unsupported_concurrency_and_unknown_body_shapes_fail_closed() {
    let s=source(MAIN);
    for bad in [
        s.replace("return await increment(counter);", "spawn Worker(); return await increment(counter);"),
        s.replace("return lease.value;", "return unknown.value;"),
        s.replace("lease.value += 1;", "lease.value *= 2;"),
        s.replace("new Mutex<int>(0)", "new Mutex<int>(get_dynamic());"),
        s.replace("using await counter.lock_async()", "using await counter.other_async()"),
        format!("{s} pub fnc external(): int {{ return 9; }}"),
    ] {
        assert!(compile(&bad).is_err(), "unsupported shape compiled: {bad}");
    }
}

#[test]
fn exceptional_and_suspending_lease_bodies_are_diagnostics() {
    let s=source(MAIN);
    for (src,code) in [
        (s.replace("lease.value += 1;", "lease.value += await 42;"),"E_GUARD_SUSPEND"),
        (s.replace("return lease.value;", "return lease;"),"E_GUARD_ESCAPE"),
        (s.replace("susp fnc increment", "fnc increment"),"E_AWAIT_EFFECT"),
    ] {
        assert_eq!(compile(&src).unwrap_err().code,code,"{src}");
    }
}

#[test]
fn static_overflow_is_prohibited_before_guest_artifact() {
    let s=source(MAIN).replace("new Mutex<int>(0)", "new Mutex<int>(9223372036854775807)");
    assert_eq!(compile(&s).unwrap_err().code,"E_OVERFLOW");
}

#[test]
fn bare_mutex_resource_constructors_fail_even_without_any_lock_async_call() {
    let invalid = [
        "pub fnc main(): int { val counter = Mutex<int>(0); return 0; }",
        "pub fnc main(): int { val counter = Mutex<Result<int,string>>(0); return 0; }",
        "pub fnc main(): int { val counter = Mutex<List<Map<int,bool>>>(0); return 0; }",
    ];
    for source in invalid {
        assert_eq!(compile(source).unwrap_err().code, "E_NEW_REQUIRED", "{source}");
    }
    // The absence of a scoped lock is not permission to emit a nominal
    // Mutex guest without a scheduler. Ordinary typed class creation stays
    // fail-closed outside the explicit closed-world ready-path profile.
    for source in [
        "pub fnc main(): int { val counter = new Mutex<int>(0); return 0; }",
        "pub fnc main(): int { val counter = new Mutex<Result<int,string>>(0); return 0; }",
    ] {
        assert_eq!(compile(source).unwrap_err().code, "E_LOCK_UNSUPPORTED", "{source}");
    }
}

#[test]
fn mutex_type_references_are_not_misdiagnosed_as_bare_constructors() {
    let source = "pub fnc main(Mutex<int> counter): int { return 0; }";
    assert_ne!(compile(source).unwrap_err().code, "E_NEW_REQUIRED");
}

#[test]
fn unrelated_ownership_generics_remain_fail_closed() {
    let other="pub fnc main(): int { val counter = new Mutex<float>(1); return 0; }";
    let err=compile(other).expect_err("unsupported generic has no guest allocation");
    assert!(matches!(err.code,"E_UNSUPPORTED"|"E_UNSUPPORTED_TYPE"|"E_LOCK_UNSUPPORTED"));
}

#[test]
fn repeated_sequential_awaits_are_lowered_with_cleanup_between_calls() {
    let main=r#"
pub susp fnc main(): int {
    val counter = new Mutex<int>(0);
    val first = await increment(counter);
    return await increment(counter);
}
"#;
    let wasm=compile(&source(main)).expect("two sequential ready acquisitions");
    assert!(wasm.starts_with(b"\0asm"));
    let overflow=source(main).replace("new Mutex<int>(0)", "new Mutex<int>(9223372036854775806)");
    assert_eq!(compile(&overflow).unwrap_err().code,"E_OVERFLOW");
}


#[test]
fn typed_task_cancel_with_mutex_remains_fail_closed() {
    // The single-task lock-elision profile MUST NOT be widened into a
    // cancellable guest TaskScope merely because its lexical body is valid.
    // The host-only TaskScopeLockCoordinator is not an installed scheduler.
    let cases = [
        r#"
pub susp fnc main(): int {
    val Future<int> work = task<int> {
        val mutex = new Mutex<int>(41);
        using await mutex.lock_async() as guard { return guard.value; }
    };
    work.cancel();
    return 0;
}
"#,
        r#"
pub susp fnc main(): int {
    do task<void> {
        val mutex = new Mutex<int>(0);
        using await mutex.lock_async() as guard { guard.value += 1; }
    };
    return 0;
}
"#,
        r#"
pub susp fnc main(): int {
    val mutex = new Mutex<int>(0);
    val Future<int> work = task<int> { return 42; };
    using mutex.lock() as guard {
        return await work;
    }
}
"#,
    ];
    for src in cases {
        assert!(compile(src).is_err(),
            "unverified cancellable TaskScope/Mutex guest was emitted: {src}");
    }
}
