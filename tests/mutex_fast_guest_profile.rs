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
