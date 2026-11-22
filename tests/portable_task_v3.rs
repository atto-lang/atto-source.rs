//! Source-compiled, strictly opt-in portable guest v3 AOT profile.
//! Real guest execution and lifecycle are exercised by portable-v3-e2e.mjs.
use atto_compiler::compile;

fn custom_section_payload(wasm:&[u8], wanted:&str)->Option<Vec<u8>> {
    fn leb(bytes:&[u8], p:&mut usize)->Option<usize> {
        let mut out=0usize;
        for shift in (0..35).step_by(7) {
            let b=*bytes.get(*p)?;*p+=1;
            out|=((b&0x7f) as usize)<<shift;
            if b&0x80==0{return Some(out)}
        }
        None
    }
    let mut p=8;
    while p<wasm.len() {
        let ty=*wasm.get(p)?;p+=1;
        let len=leb(wasm,&mut p)?;
        let end=p.checked_add(len)?;
        if end>wasm.len(){return None;}
        if ty==0 {
            let name_len=leb(wasm,&mut p)?;
            let name_end=p.checked_add(name_len)?;
            if name_end<=end && wasm.get(p..name_end)?==wanted.as_bytes(){
                return Some(wasm[name_end..end].to_vec());
            }
        }
        p=end;
    }
    None
}

#[test]
fn compiles_deterministic_real_v3_exports_only_for_closed_scalar_profile() {
    let src="pub susp fnc main(): int { await 41; return await 42; }";
    let wasm=compile(src).expect("checked source to guest Wasm");
    assert_eq!(&wasm[..8],b"\0asm\x01\0\0\0");
    assert_eq!(custom_section_payload(&wasm,"atto.browser.guest.v3"),Some(vec![3]));
    assert_eq!(custom_section_payload(&wasm,"atto.async"),Some(vec![2]));
    for name in [b"task_start_v3".as_slice(),b"task_resume_v3",
        b"task_cancel_v3",b"task_drop_v3",b"task_result_v3"] {
        assert!(wasm.windows(name.len()).any(|x|x==name),
            "compiler-generated v3 export must exist");
    }
    assert_eq!(wasm,compile(src).unwrap(),"deterministic AOT binary");
}

#[test]
fn never_advertise_portable_v3_for_unlowered_await_forms() {
    for src in [
        "pub susp fnc main(): int { val x=41; return await x + 1; }",
        "pub susp fnc main(): int { return await 42 + 1; }",
        "pub susp fnc main(): int { val f=task {return 42;}; return await f; }",
        "pub susp fnc main(): int { if true then return await 42; fi return 41; }",
        "pub susp fnc main(): int { await 42; await 43; return 44; }",
    ] {
        if let Ok(wasm)=compile(src) {
            assert_eq!(custom_section_payload(&wasm,"atto.browser.guest.v3"),None,
                "unsupported code must never claim portable continuation parity: {src}");
        }
    }
}

#[test]
fn compiles_live_scalar_local_spills_across_multiple_awaits() {
    for src in [
        "pub susp fnc main(): int { val x: int = 42; await 41; return await x; }",
        "pub susp fnc main(): int { val a: int = 41; await a; val b: int = 42; return await b; }",
        "pub susp fnc main(): int { val a: int = 41; val b: int = a; await b; return await a; }",
    ] {
        let wasm=compile(src).expect("scalar local spill must compile");
        assert_eq!(custom_section_payload(&wasm,"atto.browser.guest.v3"),Some(vec![3]),"{src}");
        assert_eq!(wasm,compile(src).unwrap(),"deterministic source-generated frames");
    }
}
#[test]
fn never_advertise_v3_for_non_scalar_or_unevaluated_local_expression() {
    for src in [
        "pub susp fnc main(): int { val flag: bool = true; await 41; return await 42; }",
        "pub susp fnc main(): int { val a: int = 40 + 2; await 41; return await a; }",
        "pub susp fnc main(): int { val a: int = 41; await a + 1; return await a; }",
        "pub susp fnc main(): int { val a: int = 41; return await a + 1; }",
    ] {
        if let Ok(wasm)=compile(src) {
            assert_eq!(custom_section_payload(&wasm,"atto.browser.guest.v3"),None,
                "unsupported local expression cannot claim v3: {src}");
        }
    }
}
