//! Explicit opt-in portable-v4 AOT frame arena compiler admission tests.
use atto_compiler::{compile,compile_portable_v4};

fn section_payload(wasm:&[u8],wanted:&str)->Option<Vec<u8>> {
    fn leb(bytes:&[u8],p:&mut usize)->Option<usize> {
        let mut out=0usize;
        for shift in (0..35).step_by(7) {
            let b=*bytes.get(*p)?;*p+=1;
            out|=((b&0x7f) as usize)<<shift;
            if b&0x80==0{return Some(out);}
        }
        None
    }
    let mut p=8;
    while p<wasm.len() {
        let ty=*wasm.get(p)?;p+=1;
        let len=leb(wasm,&mut p)?;let end=p.checked_add(len)?;
        if end>wasm.len(){return None;}
        if ty==0 {
            let n=leb(wasm,&mut p)?;let next=p.checked_add(n)?;
            if next<=end&&wasm.get(p..next)?==wanted.as_bytes(){
                return Some(wasm[next..end].to_vec());
            }
        }
        p=end;
    }
    None
}

#[test]
fn opt_in_v4_is_deterministic_and_does_not_change_v3_compilation() {
    let src="pub susp fnc main(): int { val x: int=42; await 41; return await x; }";
    let before=compile(src).unwrap();
    let v4=compile_portable_v4(src).expect("typed scalar frame is admitted");
    let after=compile(src).unwrap();
    assert_eq!(before,after,"v3 bytes unchanged by explicit v4 emission");
    assert_eq!(section_payload(&before,"atto.browser.guest.v3"),Some(vec![3]));
    assert_eq!(section_payload(&before,"atto.browser.guest.v4"),None);
    assert_eq!(section_payload(&v4,"atto.browser.guest.v3"),None);
    assert_eq!(section_payload(&v4,"atto.browser.guest.v4"),Some(vec![4]));
    assert_eq!(section_payload(&v4,"atto.async"),Some(vec![2]));
    assert_eq!(v4,compile_portable_v4(src).unwrap(),"deterministic AOT output");
    for name in [
        "task_start_v4","task_resume_v4","task_cancel_v4",
        "task_drop_v4","task_result_v4","task_capacity_v4"
    ] {
        assert!(v4.windows(name.len()).any(|w|w==name.as_bytes()),
            "missing exported lifecycle function: {name}");
    }
}
#[test]
fn unsupported_v4_shapes_fail_closed_with_diagnostic() {
    for src in [
        "pub susp fnc main(): int { val x: int=41+1; await 41; return await x; }",
        "pub susp fnc main(): int { if true then return await 42; fi return 1; }",
        "pub susp fnc main(): int { val x: bool=true; await 41; return await 42; }",
        "pub susp fnc main(): int { val f=task {return 42;}; return await f; }",
        "pub susp fnc main(): int { await 41; return 42; }",
    ] {
        assert!(compile_portable_v4(src).is_err(),
            "unimplemented v4 control/typed-future form must not emit guest: {src}");
    }
}
