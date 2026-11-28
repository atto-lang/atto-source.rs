//! Map/Record syntax discrimination and quote safety. Map guest is fail-closed.
use atto_compiler::{
    ast::{ExprKind,Program,Stmt},
    collection_types::{self,LiteralType as T},
    compile,lexer,parser::Parser,
};
fn parse(s: &str)->Result<Program,atto_compiler::Diagnostic>{
    let source=format!("pub fnc main():int {{ {s} return 0; }}");
    Parser::new(lexer::lex(&source)?).parse_program()
}
fn reject(src: &str,code: &str){
    let e=compile(src).expect_err("unsupported map must emit no guest");
    assert_eq!(e.code,code,"{src}: {e}");
}
#[test]
fn quoted_colon_and_arrow_are_maps_in_three_delimiters(){
    for literal in [
        r#"{"foo":5}"#,r#"{'foo':5}"#,r#"{`foo`:5}"#,
        r#"{"foo"=>5}"#,r#"{'foo'=>5}"#,r#"{`foo`=>5}"#
    ]{
        let p=parse(&format!("let m={literal};")).unwrap();
        let Stmt::Bind{init,..}=&p.funcs[0].body[0] else{panic!("bind")};
        let ExprKind::Map(items)=&init.kind else{panic!("not map: {literal}")};
        assert_eq!(items.len(),1);
        assert!(matches!(&items[0].0.kind,ExprKind::Str(s) if s=="foo"));
        assert!(matches!(&items[0].1.kind,ExprKind::Int(Some(5))));
        assert_eq!(collection_types::analyze(&p).unwrap()[0].value.ty,
            T::Map(Box::new(T::String),Box::new(T::Int)));
    }
    let p=parse("let r={foo:5}; let s=(\"foo\":5);").unwrap();
    assert!(matches!(&p.funcs[0].body[0],Stmt::Bind{init,..} if matches!(&init.kind,ExprKind::Record(_))));
    assert!(matches!(&p.funcs[0].body[1],Stmt::Bind{init,..} if matches!(&init.kind,ExprKind::DynRecord(_))));
}
#[test]
fn inner_quotes_and_backtick_literals_are_preserved(){
    for literal in [r#"'\"foo\"'"#, "`\"foo\"`"] {
        let p=parse(&format!("let m={{{literal}:5}};")).unwrap();
        let Stmt::Bind{init,..}=&p.funcs[0].body[0] else{panic!("bind")};
        assert!(matches!(&init.kind,ExprKind::Map(v)
            if matches!(&v[0].0.kind,ExprKind::Str(s) if s=="\"foo\"")));
    }
    let p=parse(r#"let m={`${not_evaluated}`:5};"#).unwrap();
    let Stmt::Bind{init,..}=&p.funcs[0].body[0] else{panic!("bind")};
    assert!(matches!(&init.kind,ExprKind::Map(v)
        if matches!(&v[0].0.kind,ExprKind::Str(s) if s=="${not_evaluated}")));
    let p=parse(r#"let m={`one\n`:5};"#).unwrap();
    let Stmt::Bind{init,..}=&p.funcs[0].body[0] else{panic!("bind")};
    assert!(matches!(&init.kind,ExprKind::Map(v)
        if matches!(&v[0].0.kind,ExprKind::Str(s) if s==r"one\n")));
    let p=parse(r#"let m={`one\`two`:5};"#).unwrap();
    let Stmt::Bind{init,..}=&p.funcs[0].body[0] else{panic!("bind")};
    assert!(matches!(&init.kind,ExprKind::Map(v)
        if matches!(&v[0].0.kind,ExprKind::Str(s) if s=="one`two")));
}
#[test]
fn mixed_map_entry_spellings_preserve_type_and_source_order(){
    let p=parse(r#"let m={"a":1,'b'=>2,`c`:3};"#).unwrap();
    let b=collection_types::analyze(&p).unwrap();
    assert_eq!(b[0].value.ty,T::Map(Box::new(T::String),Box::new(T::Int)));
    assert_eq!(b[0].value.children.iter().map(|x|x.ty.clone()).collect::<Vec<_>>(),
        vec![T::String,T::Int,T::String,T::Int,T::String,T::Int]);
}
#[test]
fn map_record_mix_and_wrong_context_reject(){
    for literal in [
        r#"{foo:1,"bar":2}"#,r#"{'foo':1,bar:2}"#,
        r#"{`foo`:1,bar:2}"#,r#"{foo:1,'bar'=>2}"#,
        r#"{"foo"=>1,bar:2}"#,
    ]{
        reject(&format!("pub fnc main():int {{let x={literal};return 0;}}"),"E_LITERAL_KIND");
    }
    reject(r#"pub fnc main():int {let r:Record={"foo":5};return 0;}"#,"E_TYPE");
    reject(r#"pub fnc main():int {let r:Map<string,int>={foo:5};return 0;}"#,"E_TYPE");
}
#[test]
fn string_duplicate_identity_cross_delimiters(){
    for literal in [
        r#"{"foo":1,'foo'=>2}"#,r#"{'foo':1,`foo`:2}"#,
        r#"{`foo`:1,"foo"=>2}"#,r#"{"f\u{6f}o":1,'foo':2}"#,
        r#"{"":1,'':2}"#,
    ]{
        reject(&format!("pub fnc main():int {{let m={literal};return 0;}}"),"E_DUPLICATE");
    }
    assert!(parse(r#"let m={"Foo":1,'foo':2};"#).is_ok());
}
#[test]
fn malformed_literals_and_guest_map_are_never_executed(){
    for literal in ["`unterminated","'unterminated","\"unterminated",
                    "`a\nb`","`bad\\`"]{
        reject(&format!("pub fnc main():int {{let m={{{literal}:5}};return 0;}}"),"E_STRING");
    }
    let huge="a".repeat(4097);
    reject(&format!("pub fnc main():int {{let m={{`{huge}`:5}};return 0;}}"),"E_SOURCE_LIMIT");
    for literal in [r#"{"foo":5}"#,r#"{'foo':5}"#,r#"{`foo`:5}"#,r#"{"foo"=>5}"#]{
        reject(&format!("pub fnc main():int {{let m={literal};return 0;}}"),"E_UNSUPPORTED");
    }
}

#[test]
fn nul_and_raw_carriage_returns_cannot_hide_in_quoted_map_keys(){
    for literal in ["\"before\0after\"", "'before\0after'", "`before\0after`"] {
        reject(&format!("pub fnc main():int {{let m={{{literal}:5}};return 0;}}"),
            "E_NUL");
    }
    reject("pub fnc main():int {let m={`before\rafter`:5};return 0;}",
        "E_STRING");
}