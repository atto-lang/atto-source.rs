//! Standalone Entry and Symbol static-typing integration on top of real bounded
//! Map/Pair/Entry guests. A type witness is not guest allocation.
use atto_compiler::{
    ast::{ExprKind, Program, Stmt},
    collection_types::{self, LiteralType as T},
    compile, lexer, parser::Parser,
};

fn parse(body: &str) -> Result<Program, atto_compiler::Diagnostic> {
    let s = format!("pub fnc main():int {{ {body} return 0; }}");
    Parser::new(lexer::lex(&s)?).parse_program()
}
fn init(p: &Program, i: usize) -> &ExprKind {
    match &p.funcs[0].body[i] {
        Stmt::Bind { init, .. } => &init.kind,
        x => panic!("expected binding, got {x:?}"),
    }
}
fn rejected(src: &str, code: &str) {
    let error=compile(&format!("pub fnc main():int {{ {src} return 0; }}"))
        .expect_err("unverified guest operation must emit no artifact");
    assert_eq!(error.code,code,"{src}: {error}");
}
fn admitted(source: &str) {
    let bytes=compile(source).unwrap_or_else(|e| panic!("{source}\n{e}"));
    assert!(bytes.starts_with(b"\0asm\x01\0\0\0"));
    assert_eq!(bytes,compile(source).unwrap());
}

#[test]
fn standalone_entries_have_distinct_symbol_and_string_type_identity() {
    let p=parse(r#"let a="foo"=>5;let b=#foo=>5;"#).unwrap();
    assert!(matches!(init(&p,0),ExprKind::Entry(k,_)
        if matches!(&k.kind,ExprKind::Str(s) if s=="foo")));
    assert!(matches!(init(&p,1),ExprKind::Entry(k,_)
        if matches!(&k.kind,ExprKind::Symbol(s) if s=="foo")));
    let ts=collection_types::analyze(&p).unwrap();
    assert_eq!(ts[0].value.ty,T::Entry(Box::new(T::String),Box::new(T::Int)));
    assert_eq!(ts[1].value.ty,T::Entry(Box::new(T::Symbol),Box::new(T::Int)));
}

#[test]
fn typed_pair_entry_shorthand_and_bounded_map_factory_still_emit_guest() {
    admitted(r#"pub fnc main():int {
        val Pair<int,bool> p=40=>true;
        val Entry<int,bool> e=2=>false;
        return p.first+e.key;
    }"#);
    admitted(r#"pub fnc main():int {
        val m=Map.of_entries<int,bool>(5=>true,Entry.of(8,false));
        return m[5]?42:0;
    }"#);
    admitted(r#"pub fnc main():int {
        val m=Map.of("a"=>40,"b"=>2);
        return m["a"]+m["b"];
    }"#);
    admitted(r#"pub fnc main():int {
        val m=Map.of("a",40,"b",2);
        return m["a"]+m["b"];
    }"#);
}

#[test]
fn bracket_key_remains_list_not_computed_scalar_marker() {
    for body in [r#"let x=["foo"]=>5;"#, "let x=[true]=>5;"] {
        let p=parse(body).unwrap();
        assert!(matches!(init(&p,0),ExprKind::Entry(k,_)
            if matches!(&k.kind,ExprKind::Array(_))));
        assert_eq!(collection_types::analyze(&p).unwrap_err().code,"E_MAP_KEY");
    }
    let p=parse(r#"let m={["foo"]=>5};"#).unwrap();
    assert!(matches!(init(&p,0),ExprKind::Map(xs)
        if matches!(&xs[0].0.kind,ExprKind::Array(_))));
    assert_eq!(collection_types::analyze(&p).unwrap_err().code,"E_MAP_KEY");
}

#[test]
fn scalar_symbol_maps_are_type_only_and_guest_denied() {
    let p=parse(r#"let Entry<Symbol,int> e=#foo=>5;let Map<Symbol,int> m={#foo=>5,#bar=>7};"#).unwrap();
    let t=collection_types::analyze(&p).unwrap();
    assert_eq!(t[0].value.ty,T::Entry(Box::new(T::Symbol),Box::new(T::Int)));
    assert_eq!(t[1].value.ty,T::Map(Box::new(T::Symbol),Box::new(T::Int)));
    assert_eq!(parse("let m={#foo=>1,#foo=>2};").unwrap_err().code,"E_DUPLICATE");
    rejected("let m={#foo=>5};","E_MAP_KEY");
    rejected("let e=#foo=>5;","E_UNSUPPORTED");
    rejected(r#"let e="foo"=>5;"#,"E_UNSUPPORTED");
}

#[test]
fn ambiguous_or_unhashable_entries_and_symbol_lexer_reject() {
    for s in [r#"let x=["a"]=>5;"#, "let x=[true]=>5;"] {
        let p=parse(s).unwrap();
        assert_eq!(collection_types::analyze(&p).unwrap_err().code,"E_MAP_KEY");
    }
    assert_eq!(parse("let x=#foo=>5=>7;").unwrap_err().code,"E_LITERAL_KIND");
    assert_eq!(parse("let x=bar::5;").unwrap_err().code,"E_SYNTAX");
    for text in ["#","#123","# foo","##foo"] {
        assert_eq!(lexer::lex(text).unwrap_err().code,"E_SYMBOL");
    }
    let too_long="a".repeat(4097);
    assert_eq!(lexer::lex(&format!("#{too_long}")).unwrap_err().code,"E_SOURCE_LIMIT");
}

#[test]
fn unknown_runtime_key_does_not_acquire_ambient_hash_authority() {
    let p=parse(r#"let name="foo";let e=name=>5;"#).unwrap();
    assert_eq!(collection_types::analyze(&p).unwrap_err().code,"E_UNSUPPORTED");
    rejected(r#"let name="foo";let m={name=>5};"#,"E_UNSUPPORTED");
}
