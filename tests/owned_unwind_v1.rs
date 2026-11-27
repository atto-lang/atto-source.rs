use atto_compiler::{compile,compile_owned_unwind_v1 as emit};
const DEFER:&str=include_str!("fixtures/owned-pending-defer.atto");
const RECOVER:&str=include_str!("fixtures/owned-pending-recover.atto");
#[test]
fn exact_owned_fixtures_emit_deterministic_wasm_for_all_effect_aliases(){
    for source in [DEFER,RECOVER]{
        let b=emit(source).unwrap();
        assert!(b.starts_with(b"\0asm\x01\0\0\0"));
        for alias in ["susp","⧖","∂"]{assert_eq!(emit(&source.replace("suspends",alias)).unwrap(),b);}
        assert_eq!(emit(source).unwrap(),b);
        assert_eq!(compile(source).unwrap_err().code,"E_UNSUPPORTED_TYPE");
    }
}
#[test]
fn denied_shapes_produce_no_guest(){
    for s in [RECOVER.replace("take! ",""),RECOVER.replace("take! Future","take ! Future"),RECOVER.replace("take! Future","take!Future"),RECOVER.replace("await fallback","await input"),RECOVER.replace("err, cursor","input, cursor"),RECOVER.replace("await fallback","await fallback + 1"),RECOVER.replace("Future<int> fallback","Future<bool> fallback"),RECOVER.replace("suspends ",""),RECOVER.replace("return await input","return 42"),format!("{RECOVER} pub fnc main():int{{return 1;}}")]{
        let e=emit(&s).expect_err("unsafe source emitted");assert!(e.pos.line>0&&e.pos.col>0);
    }
}
