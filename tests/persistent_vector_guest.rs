//! Bounded immutable Vector copy-update (not general path-copy persistent runtime).
use atto_compiler::compile;

fn accepted(code: &str) {
    let bytes=compile(code).unwrap_or_else(|e| panic!("{code}\n{e}"));
    assert!(bytes.starts_with(b"\0asm\x01\0\0\0"));
    assert_eq!(bytes,compile(code).unwrap(),"Wasm output must be deterministic");
}
fn rejected(code: &str,expected: &str) {
    let err=compile(code).expect_err("invalid persistent update produced guest bytes");
    assert_eq!(err.code,expected,"{code}\n{err}");
}

#[test]
fn append_and_replace_create_new_guest_values() {
    accepted("pub fnc main(): int {const Vector<int> a=[];val Vector<int> b=a.appended(42);return b[0];}");
    accepted("pub fnc main(): int {val Vector<int> a=[10,20];val Vector<int> b=a.appended(12);return a[1]+b[2]+10;}");
    accepted("pub fnc main(): int {val Vector<int> a=[2];val Vector<int> b=a.updated(0,42);return a[0]+b[0]-2;}");
    accepted("pub fnc main(): int {val Vector<int> a=[1,2];val Vector<int> b=a.appended(3);val Vector<int> c=b.updated(0,39);return a[0]+c[0]+c[2]-1;}");
}

#[test]
fn static_updates_reject_dynamic_or_invalid_mutation() {
    rejected("pub fnc main(): int {val Vector<int> a=[1];val Vector<int> b=a.updated(1,42);return 0;}","E_INDEX_BOUNDS");
    rejected("pub fnc main(): int {val Vector<int> a=[];val Vector<int> b=a.updated(0,42);return 0;}","E_INDEX_BOUNDS");
    rejected("pub fnc main(): int {val Vector<int> a=[1];val int index=0;val Vector<int> b=a.updated(index,42);return 0;}","E_UNSUPPORTED");
    rejected("pub fnc main(): int {val Vector<int> a=[1];val int value=42;val Vector<int> b=a.appended(value);return 0;}","E_UNSUPPORTED");
    rejected("pub fnc main(): int {val Vector<int> a=[1,2,3,4,5,6,7,8];val Vector<int> b=a.appended(9);return 0;}","E_UNSUPPORTED");
    rejected("pub fnc main(): int {val Vector<int> a=[1];val Vector<int> b=a.appended(2,3);return 0;}","E_FACTORY_ARITY");
    rejected("pub fnc main(): int {val Vector<int> a=[1];val Vector<int> b=a.updated(0,\"bad\");return 0;}","E_TYPE");
}

#[test]
fn runtime_and_type_identity_fail_closed() {
    rejected("pub fnc main(): int {val List<int> a=[1];val Vector<int> b=a.appended(2);return 0;}","E_UNSUPPORTED");
    rejected("pub fnc main(): int {val Vector<int> a=[1];val List<int> b=a.appended(2);return 0;}","E_TYPE");
}
