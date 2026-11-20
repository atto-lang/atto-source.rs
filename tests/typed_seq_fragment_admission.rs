//! Host-only typed std.pattern fragment admission tests. No Atto guest executed.
use atto_compiler::sequence_pattern_fragments::{
    compile_seq_part, compose, reference_match_i64, Atom, ElementType as T, SeqPart, Source,
};

fn parse(text: &str, ty: T) -> SeqPart {
    compile_seq_part(true, true, Source::Literal(text), ty, None).unwrap()
}
fn rejected(source: &str, ty: T, code: &str) {
    let err = compile_seq_part(true, true, Source::Literal(source), ty, None).unwrap_err();
    assert_eq!(err.code, code, "pattern={source:?}: {err:?}");
}

#[test]
fn explicit_and_contextual_element_types_agree() {
    assert_eq!(
        compile_seq_part(true, true, Source::Literal("..."), T::Int, Some(T::Int)).unwrap(),
        parse("...", T::Int)
    );
    assert_eq!(parse("...",T::Int).atoms, vec![Atom::Rest]);
    assert_eq!(parse("_",T::Any).atoms, vec![Atom::One]);
    assert_eq!(parse("...(0 of int), 7",T::Int).atoms, vec![Atom::Int(7)]);
    assert_eq!(parse("...(3 of int)",T::Int).atoms,
        vec![Atom::Is{ty:T::Int,negative:false};3]);
}

#[test]
fn type_tests_are_explicit_and_not_implicit_wildcard_casts() {
    assert_eq!(parse("is int",T::Any).atoms,
        vec![Atom::Is{ty:T::Int,negative:false}]);
    assert_eq!(parse("is !bool",T::Int).atoms,
        vec![Atom::Is{ty:T::Bool,negative:true}]);
    rejected("is bool",T::Int,"E_PATTERN_TYPE");
    rejected("is !int",T::Int,"E_PATTERN_TYPE");
    rejected("is NonReifiable<T>",T::Any,"E_PATTERN_TYPE");
    rejected("is int as val x",T::Any,"E_PATTERN_BINDING");
    rejected("-0",T::Int,"E_PATTERN_SOURCE");
    rejected("7",T::String,"E_PATTERN_TYPE");
    rejected("true",T::Int,"E_PATTERN_TYPE");
    assert_eq!(parse("true",T::Any).atoms,vec![Atom::Bool(true)]);
    assert_eq!(parse("\"a,b\"",T::String).atoms, vec![Atom::String("a,b".into())]);
}

#[test]
fn counted_groups_are_fixed_and_bounded() {
    assert_eq!(parse("...(2 of 7), ...(1 of _)",T::Int).atoms,
        vec![Atom::Int(7),Atom::Int(7),Atom::One]);
    for text in [
        "...(2 of ...)", "...(2 of ...(1 of int))",
        "...(3 of List.of(1))", "...(0 of is int as val x)",
        "...(01 of int)", "...(2..4 of int)", "...(n of int)",
        "...(9999999999999999999999 of int)"
    ] { assert!(parse_rejects(text,T::Int), "{text}"); }
    rejected(&format!("...(129 of int)"),T::Int,"E_PATTERN_BUDGET");
}

fn parse_rejects(text: &str, ty: T) -> bool {
    compile_seq_part(true,true,Source::Literal(text),ty,None).is_err()
}

#[test]
fn identity_and_constexpr_provenance_gate_intrinsic() {
    let parse_as = |trusted,place,source,ty| compile_seq_part(trusted,place,source,ty,None);
    assert_eq!(parse_as(false,true,Source::Literal("..."),T::Int).unwrap_err().code,"E_PATTERN_CONTEXT");
    assert_eq!(parse_as(true,false,Source::Literal("..."),T::Int).unwrap_err().code,"E_PATTERN_CONTEXT");
    assert_eq!(parse_as(true,true,Source::Runtime,T::Int).unwrap_err().code,"E_PATTERN_CONSTEXPR");
    assert_eq!(parse_as(true,true,Source::ProvenPureConst("..."),T::Int).unwrap(),parse("...",T::Int));
    assert_eq!(compile_seq_part(true,true,Source::Literal("..."),T::Any,Some(T::Int))
        .unwrap_err().code,"E_PATTERN_TYPE");
}

#[test]
fn arbitrary_calls_bindings_and_unsafe_unicode_are_not_grammar() {
    for source in [
        "is int as val x", "get_secret()", "await read()",
        "pattern.parse_seq<int>(user)", "1, ", "..., ...",
        "..._", "_...", "1 as val x", "1, (2, 3)", "...(2 of int",
        "\"unterminated", "\"a\\z\"", "\"a\" junk", "\u{202e}..."
    ] { assert!(parse_rejects(source,T::Any), "{source:?}"); }
    rejected("",T::Any,"E_PATTERN_SOURCE");
    rejected(&"x".repeat(4097),T::Any,"E_PATTERN_BUDGET");
}

#[test]
fn composition_detects_adjacency_and_type_erasure() {
    let rest = parse("...",T::Int);
    let value = parse("7",T::Int);
    assert_eq!(compose(&[rest.clone(),value,rest.clone()],T::Int).unwrap().atoms.len(),3);
    assert_eq!(compose(&[rest.clone(),rest],T::Int).unwrap_err().code,"E_PATTERN_SOURCE");
    assert_eq!(compose(&[parse("...",T::Any)],T::Int).unwrap_err().code,"E_PATTERN_TYPE");
}

#[test]
fn reference_match_leftmost_shortest_and_fixed_segments() {
    let pat = compose(&[
        parse("...",T::Int),parse("3, 4, 5",T::Int),parse("...",T::Int)
    ],T::Int).unwrap();
    assert_eq!(reference_match_i64(&pat,&[1,2,3,4,5,9]).unwrap(),Some(vec![(0,2),(5,6)]));
    assert_eq!(reference_match_i64(&pat,&[1,3,4,8,5]).unwrap(),None);
    assert_eq!(reference_match_i64(&pat,&[3,4,5]).unwrap(),Some(vec![(0,0),(3,3)]));
    let pat = parse("..., 3, ..., 3",T::Int);
    assert_eq!(reference_match_i64(&pat,&[3]).unwrap(),None);
    assert_eq!(reference_match_i64(&pat,&[3,3]).unwrap(),Some(vec![(0,0),(1,1)]));
}

#[test]
fn reference_match_rejects_oversized_values_and_mismatched_type() {
    let part = parse("...",T::Int);
    assert_eq!(reference_match_i64(&part,&[1;257]).unwrap_err().code,"E_PATTERN_BUDGET");
    let s = parse("...",T::String);
    assert_eq!(reference_match_i64(&s,&[1]).unwrap_err().code,"E_PATTERN_TYPE");
}
