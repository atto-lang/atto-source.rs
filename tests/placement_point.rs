const GOOD:&str=include_str!("fixtures/pass/placement-point-reuse.atto");
#[test]
fn positive_emits_actual_memory_guest(){
    let wasm=atto_compiler::compile(GOOD).unwrap();
    assert_eq!(&wasm[..4],b"\0asm");
    assert!(wasm.windows(3).any(|w|w==[0x37,3,0]));
    assert!(wasm.windows(3).any(|w|w==[0x29,3,8]));
}
#[test]
fn lifecycle_rejects_before_artifact(){
    for (from,to) in [
        ("new Point(40, 1) at s;",""),
        ("unref! p;","p = null;"),
        ("unref! p;",""),
        ("destroy! s;","free! s;"),
        ("destroy! s;","new Point(1, 2) at s;"),
        ("free! s;",""),
        ("free! s;","free! s; free! s;"),
        ("(deref! p).x","(deref! p).z"),
        ("return first + second;","return p;"),
        ("ref! p to s;","ref ! p to s;"),
    ]{
        let e=atto_compiler::compile(&GOOD.replacen(from,to,1)).expect_err("invalid placement compiled");
        assert_eq!(e.code,"E_POINTER_PLACEMENT","{from} -> {to}");
    }
}
