//! Bounded guest-Wasm closed Record compilation contract.
//! Node's record_with_guest.mjs executes the emitted guest programs.
use atto_compiler::{compile, lexer, parser::Parser, record_guest};

fn pass(src: &str) {
    let wasm=compile(src).unwrap_or_else(|d|panic!("expected compiled guest: {d}"));
    assert!(wasm.starts_with(b"\0asm\x01\0\0\0"),"invalid wasm header");
    assert_eq!(wasm,compile(src).unwrap(),"compiled module must be deterministic");
}
fn fail(src: &str, code: &str) {
    let d=compile(src).expect_err("unsafe input must not produce a guest module");
    assert_eq!(d.code,code,"{src}: {d}");
}

#[test]
fn runs_same_shape_copy_without_mutating_source() {
    pass("fnc main(): int { val r={a:7,b:5}; val s=r with {a:10}; return r.a*100+s.a*10+s.b; }");
}
#[test]
fn shape_growth_and_type_change_are_compilable() {
    pass(r#"fnc main(): int {
        val original={age:30};
        val updated=original with {age:"thirty",active:true};
        if updated.age eq "thirty" then
            return original.age + 1;
        else
            return 0;
        fi
    }"#);
}
#[test]
fn chained_and_parenthesized_with_are_compilable() {
    pass("fnc main(): int { val v=({a:1,b:2} with {a:4}) with {c:5}; return v.a*100+v.b*10+v.c; }");
}
#[test]
fn structural_shape_is_interned_independent_of_declaration_order() {
    let p=Parser::new(lexer::lex("fnc main(): int {
        val a={x:1,y:true};
        val b={y:false,x:2};
        return a.x+b.x;
    }").unwrap()).parse_program().unwrap();
    let lowered=record_guest::lower(&p).unwrap();
    assert_eq!(lowered.classes.len(),1,"same canonical structural shape must share one class layout");
    pass("fnc main(): int { val a={x:1,y:true}; val b={y:false,x:2}; return a.x+b.x; }");
}
#[test]
fn empty_update_builds_an_independent_root() {
    pass("fnc main(): int { val r={a:42}; val s=r with {}; return r.a+s.a; }");
}
#[test]
fn chained_with_reads_original_after_copy() {
    pass("fnc main(): int { val r={a:3,b:4}; val s=r with {a:5} with {b:7}; return r.a*100+s.a*10+s.b; }");
}
#[test]
fn record_annotation_is_checked_before_guest_erasure() {
    pass("fnc main(): int { val r: Record={a:20}; val s: record=r with {b:22}; return r.a+s.b; }");
}
#[test]
fn reject_quoted_punctuation_until_guest_bracket_layout_is_verified() {
    fail(r#"fnc main(): int { val x={"content-type":"json"}; return 1; }"#,"E_UNSUPPORTED");
}
#[test]
fn reject_nested_object_ownership_and_dynamic_value() {
    fail("fnc main(): int { val r={a:{nested:1}}; return 0; }","E_UNSUPPORTED");
    fail("fnc main(): int { val r={a:1}; val s=r with {b:unknown()}; return 0; }","E_WITH_TYPE");
}
#[test]
fn reject_unproven_open_record_and_wrong_destination_type() {
    fail("fnc update(r:Record): int { val x=r with {a:1}; return 0; } fnc main():int{return 0;}","E_WITH_OPEN_RECORD");
    fail(r#"fnc main(): int {
        val x={a:1}; val y=x with {a:"str"}; val invalid:int=y.a; return 0;
    }"#,"E_TYPE");
}
#[test]
fn duplicate_field_and_map_patch_are_not_reinterpreted() {
    fail("fnc main(): int { val r={a:1}; val s=r with {a:2,a:3};return 0; }","E_DUPLICATE");
    fail("fnc main(): int { val r={a:1}; val s=r with {a=>2};return 0; }","E_WITH_MAP");
}
#[test]
fn constant_record_allocation_is_not_misrepresented_as_const() {
    fail("fnc main(): int { const r={a:1}; return 0; }","E_CONST");
}
#[test]
fn forged_internal_record_class_is_rejected() {
    fail("fnc main(): int { val x=__atto_shape_with_record_0; return 0; }","E_RESERVED");
}
#[test]
fn boolean_and_float_fields_stay_copyable_without_aliases() {
    pass("fnc main(): int { val r={f:2.5,active:true}; val s=r with {active:false}; if r.active then return 1; else return 0; fi }");
}
#[test]
fn shape_and_field_limits_fail_closed() {
    fail("fnc main(): int { val r={a:1,b:2,c:3,d:4,e:5,f:6,g:7,h:8,i:9}; return 0; }","E_UNSUPPORTED");
}
