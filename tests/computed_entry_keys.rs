//! Staged static Entry and Symbol-key syntax. This is NOT guest Map storage.
use atto_compiler::{
    ast::{ExprKind,Program,Stmt},
    collection_types::{self,LiteralType as T},
    compile,lexer,parser::Parser,
};
fn parse(body:&str)->Result<Program,atto_compiler::Diagnostic>{
    let s=format!("pub fnc main():int {{ {body} return 0; }}");
    Parser::new(lexer::lex(&s)?).parse_program()
}
fn first_init(program:&Program, index:usize)->&ExprKind{
    match &program.funcs[0].body[index]{
        Stmt::Bind{init,..}=>&init.kind,
        other=>panic!("not a binding: {other:?}"),
    }
}
fn expected_reject(src:&str,code:&str){
    let e=compile(&format!("pub fnc main():int {{ {src} return 0; }}"))
        .expect_err("no unverified Entry/Map/Symbol guest artifact");
    assert_eq!(e.code,code,"{src}: {e}");
}
#[test]
fn inferred_symbol_and_string_entries_preserve_nominal_type(){
    let p=parse(r#"let string_entry="foo"=>5; let symbol_entry=#foo=>5;"#).unwrap();
    assert!(matches!(first_init(&p,0),ExprKind::Entry(k,v)
        if matches!(&k.kind,ExprKind::Str(s) if s=="foo")
        && matches!(&v.kind,ExprKind::Int(Some(5)))));
    assert!(matches!(first_init(&p,1),ExprKind::Entry(k,v)
        if matches!(&k.kind,ExprKind::Symbol(s) if s=="foo")
        && matches!(&v.kind,ExprKind::Int(Some(5)))));
    let ir=collection_types::analyze(&p).unwrap();
    assert_eq!(ir.len(),2);
    assert_eq!(ir[0].value.ty,T::Entry(Box::new(T::String),Box::new(T::Int)));
    assert_eq!(ir[1].value.ty,T::Entry(Box::new(T::Symbol),Box::new(T::Int)));
    assert_ne!(ir[0].value.ty,ir[1].value.ty);
}
#[test]
fn computed_brackets_are_explicit_key_expressions_not_array_keys(){
    let p=parse(r#"const string foo="bar"; let e=[foo]=>5; let m={[foo]=>7};"#).unwrap();
    assert!(matches!(first_init(&p,1),ExprKind::Entry(k,_)
        if matches!(&k.kind,ExprKind::Var(s) if s=="foo")));
    assert!(matches!(first_init(&p,2),ExprKind::Map(kv)
        if matches!(&kv[0].0.kind,ExprKind::Var(s) if s=="foo")));
    let ir=collection_types::analyze(&p).unwrap();
    assert_eq!(ir.len(),2);
    assert_eq!(ir[0].value.ty,T::Entry(Box::new(T::String),Box::new(T::Int)));
    assert_eq!(ir[1].value.ty,T::Map(Box::new(T::String),Box::new(T::Int)));
    let old=parse(r#"const string foo="bar";let e=foo=>5;"#).unwrap();
    assert_eq!(collection_types::analyze(&old).unwrap()[0].value.ty,
        T::Entry(Box::new(T::String),Box::new(T::Int)));
    let array=parse(r#"let array=[5];"#).unwrap();
    assert!(matches!(first_init(&array,0),ExprKind::Array(v) if v.len()==1));
}
#[test]
fn typed_symbol_maps_entries_and_duplicates(){
    let p=parse(r#"let Entry<Symbol,int> e=#foo=>5; let Map<Symbol,int> m={#foo=>5,#bar=>7};"#).unwrap();
    let ir=collection_types::analyze(&p).unwrap();
    assert_eq!(ir[0].value.ty,T::Entry(Box::new(T::Symbol),Box::new(T::Int)));
    assert_eq!(ir[1].value.ty,T::Map(Box::new(T::Symbol),Box::new(T::Int)));
    assert_eq!(parse(r#"let m={#foo=>1,#foo=>2};"#).unwrap_err().code,"E_DUPLICATE");
    let mismatched=parse(r#"let Map<string,int> m={#foo=>5};"#).unwrap();
    assert_eq!(collection_types::analyze(&mismatched).unwrap_err().code,"E_TYPE");
    let mismatched=parse(r#"let Entry<string,int> e=#foo=>5;"#).unwrap();
    assert_eq!(collection_types::analyze(&mismatched).unwrap_err().code,"E_TYPE");
    let mismatched=parse(r#"let m={"foo"=>5,#foo=>7};"#).unwrap();
    assert_eq!(collection_types::analyze(&mismatched).unwrap_err().code,"E_TYPE");
}
#[test]
fn nonassociative_arrow_and_bounded_computed_key(){
    for s in ["let e=[a,b]=>5;","let e=[]=>5;","let m={[a,b]=>5};"]{
        assert_eq!(parse(s).unwrap_err().code,"E_MAP_KEY", "{s}");
    }
    assert_eq!(parse(r#"let e=#foo=>5=>7;"#).unwrap_err().code,"E_LITERAL_KIND");
    assert_eq!(parse("let e=bar::5;").unwrap_err().code,"E_SYNTAX");
    for source in ["#","#123","# foo","##foo"] {
        assert_eq!(lexer::lex(source).unwrap_err().code,"E_SYMBOL", "{source}");
    }
    let name="a".repeat(4097);
    assert_eq!(lexer::lex(&format!("#{name}")).unwrap_err().code,"E_SOURCE_LIMIT");
}
#[test]
fn unknown_or_mutable_runtime_keys_cannot_be_fake_inferred(){
    let p=parse(r#"let string foo="bar";let e=[foo]=>5;"#).unwrap();
    assert_eq!(collection_types::analyze(&p).unwrap_err().code,"E_UNSUPPORTED");
    let p=parse(r#"let e=[unbound]=>5;"#).unwrap();
    assert_eq!(collection_types::analyze(&p).unwrap_err().code,"E_UNSUPPORTED");
}
#[test]
fn compiled_symbol_entry_and_computed_map_guest_fail_closed(){
    for source in [
        r#"let e=#foo=>5;"#,
        r#"let m={#foo=>5};"#,
        r#"const string foo="bar";let e=[foo]=>5;"#,
        r#"const string foo="bar";let m={[foo]=>5};"#,
        r#"let e="foo"=>5;"#,
    ] {
        expected_reject(source,"E_UNSUPPORTED");
    }
}

#[test]
fn record_construction_remains_identifier_only_and_has_no_entry_type(){
    let p=parse(r#"let a={bar:5};let b=(bar=5);let m={"bar":5};"#).unwrap();
    assert!(matches!(first_init(&p,0),ExprKind::Record(_)));
    assert!(matches!(first_init(&p,1),ExprKind::Record(_)));
    assert!(matches!(first_init(&p,2),ExprKind::Map(_)));
    let ir=collection_types::analyze(&p).unwrap();
    assert_eq!(ir[0].value.ty,ir[1].value.ty);
    assert_ne!(ir[0].value.ty,ir[2].value.ty);
    assert_eq!(parse("let r={bar};").unwrap_err().code,"E_LITERAL_KIND");
    let p=parse(r#"const string bar="hello";let a={bar:bar};let b=(bar=bar);"#).unwrap();
    let ir=collection_types::analyze(&p).unwrap();
    assert_eq!(ir[0].value.ty,ir[1].value.ty);
    assert_eq!(ir[0].value.ty,T::Record(vec![("bar".to_owned(),T::String)]));
}
#[test]
fn lexical_consts_cannot_leak_across_scopes_or_mutable_shadow(){
    let inner=parse(r#"block {const string foo="bar";}let e=[foo]=>5;"#).unwrap();
    assert_eq!(collection_types::analyze(&inner).unwrap_err().code,"E_UNSUPPORTED");
    let shadow=parse(r#"const string foo="bar";block {let foo=7;let e=[foo]=>5;}"#).unwrap();
    assert_eq!(collection_types::analyze(&shadow).unwrap_err().code,"E_UNSUPPORTED");
    let valid=parse(r#"const string foo="bar";block {let e=[foo]=>5;}"#).unwrap();
    assert_eq!(collection_types::analyze(&valid).unwrap()[0].value.ty,
        T::Entry(Box::new(T::String),Box::new(T::Int)));
}
