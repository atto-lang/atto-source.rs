use atto_compiler::{compile,lexer,parser::Parser,ast::{Stmt,BindKind}};
#[test]
fn const_mut_is_val_not_writable_referent_or_rebind() {
    for decl in ["val x = 42","const mut x = 42"] {
        let src=format!("pub fnc main(): int {{ {decl}; return x; }}");
        let p=Parser::new(lexer::lex(&src).unwrap()).parse_program().unwrap();
        assert!(matches!(&p.funcs[0].body[0],Stmt::Bind{kind:BindKind::Val,referent_mutable:false,..}));
        assert!(compile(&src).is_ok());
        let bad=format!("pub fnc main(): int {{ {decl}; x = 1; return x; }}");
        assert_eq!(compile(&bad).unwrap_err().code,"E_IMMUTABLE");
    }
    let a=compile("pub fnc main(): int {val x=42;return x;}").unwrap();
    let b=compile("pub fnc main(): int {const mut x=42;return x;}").unwrap();
    assert_eq!(a,b);
}
#[test]
fn const_mut_does_not_grant_record_field_write() {
    for decl in ["val Record r={x:1}","const mut Record r={x:1}"] {
        let src=format!("pub fnc main(): int {{ {decl}; r.x=42; return r.x; }}");
        assert!(compile(&src).is_err(),"{src}");
    }
    assert!(compile("pub fnc main(): int {val mut Record r={x:1};r.x=42;return r.x;}").is_ok());
    assert_eq!(compile("pub fnc main(): int {val mut mut x=1;return x;}").unwrap_err().code,"E_SYNTAX");
}
