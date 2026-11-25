//! Parenthesized identifier-named Record literals share the existing closed
//! Record type checker and guest ABI; named CALL arguments are not yet admitted.
//! No parser-only result is claimed as guest execution.
use atto_compiler::{
    ast::{ExprKind, Program, Stmt},
    collection_types::{self, LiteralType},
    compile, lexer, parser::Parser,
};

fn parse(source: &str) -> Program {
    Parser::new(lexer::lex(source).unwrap())
        .parse_program()
        .unwrap_or_else(|e| panic!("{source}: {e}"))
}

fn binding(source: &str) -> Program {
    parse(&format!("pub fnc main(): int {{ {source} return 0; }}"))
}

fn rejected(source: &str, code: &str) {
    let err = compile(source).expect_err("unadmitted syntax must emit no guest Wasm");
    assert_eq!(err.code, code, "{source}: {err}");
}

fn guest(source: &str) {
    let bytes = compile(source).unwrap_or_else(|e| panic!("{source}: {e}"));
    assert!(bytes.starts_with(b"\0asm\x01\0\0\0"), "{source}");
    assert_eq!(bytes, compile(source).unwrap(), "guest codegen must be deterministic");
}

#[test]
fn tuple_grouping_empty_and_named_record_grammars_do_not_alias() {
    let p = binding(
        "let a=(); let b=(42); let c=(42,); let d=(42,true); \
         let e=(foo=42); let f=(foo=42,); let g=(foo=42,bar=true,); \
         let h={foo:42,bar:true}; let i={\"foo\" => 42}; let j={};"
    );
    let b = &p.funcs[0].body;
    assert!(matches!(&b[0], Stmt::Bind {init,..}
        if matches!(&init.kind,ExprKind::Tuple(items) if items.is_empty())));
    assert!(matches!(&b[1], Stmt::Bind {init,..}
        if matches!(&init.kind,ExprKind::Int(Some(42)))));
    assert!(matches!(&b[2], Stmt::Bind {init,..}
        if matches!(&init.kind,ExprKind::Tuple(items) if items.len()==1)));
    assert!(matches!(&b[3], Stmt::Bind {init,..}
        if matches!(&init.kind,ExprKind::Tuple(items) if items.len()==2)));
    for (i, stmt) in b.iter().enumerate().take(8).skip(4) {
        assert!(matches!(stmt, Stmt::Bind {init,..}
            if matches!(&init.kind,ExprKind::Record(_))), "record at {i}");
    }
    assert!(matches!(&b[8], Stmt::Bind {init,..}
        if matches!(&init.kind,ExprKind::Map(_))));
    assert!(matches!(&b[9], Stmt::Bind {init,..}
        if matches!(&init.kind,ExprKind::EmptyBraces)));
}

#[test]
fn parenthesized_record_and_braced_record_share_exact_typed_shape() {
    let p = binding("let a=(z=true,foo=42); let b={foo:42,z:true};");
    let typed = collection_types::analyze(&p).unwrap();
    assert_eq!(typed.len(), 2);
    assert_eq!(typed[0].value.ty, typed[1].value.ty);
    assert_eq!(typed[0].value.ty, LiteralType::Record(vec![
        ("foo".into(),LiteralType::Int),
        ("z".into(),LiteralType::Bool)
    ]));
    assert_eq!(typed[0].value.source_to_record_slot, vec![1,0]);
    assert_eq!(typed[1].value.source_to_record_slot, vec![0,1]);
    assert_eq!(typed[0].value.children.len(),2);
}

#[test]
fn inferred_named_record_and_explicit_record_type_are_identical() {
    let p = binding("let a=(name=\"star\",count=5); let b: Record=(name=\"star\",count=5);");
    let typed = collection_types::analyze(&p).unwrap();
    assert_eq!(typed.len(),2);
    assert_eq!(typed[0].value.ty, typed[1].value.ty);
    assert_eq!(typed[0].value.ty, LiteralType::Record(vec![
        ("count".into(),LiteralType::Int),
        ("name".into(),LiteralType::String)
    ]));
}

#[test]
fn empty_tuple_is_never_a_record_and_bare_braces_need_context() {
    let p = binding("let t=(); let r: Record={}; let m: Map<string,int>={};");
    let typed = collection_types::analyze(&p).unwrap();
    assert_eq!(typed.len(),3);
    assert_eq!(typed[0].value.ty,LiteralType::Tuple(vec![]));
    assert_eq!(typed[1].value.ty,LiteralType::Record(vec![]));
    assert_eq!(typed[2].value.ty,LiteralType::Map(
        Box::new(LiteralType::String),Box::new(LiteralType::Int)));
    let p = binding("let x={};");
    assert_eq!(collection_types::analyze(&p).unwrap_err().code,"E_INFER");
    let p = binding("let x: Record=();");
    assert_eq!(collection_types::analyze(&p).unwrap_err().code,"E_TYPE");
    let p = binding("let x: Tuple={};");
    assert_eq!(collection_types::analyze(&p).unwrap_err().code,"E_INFER");
}

#[test]
fn named_record_mixes_duplicates_and_quoted_labels_reject_before_wasm() {
    for (expr, code) in [
        ("(foo=1,foo=2)","E_DUPLICATE"),
        ("(foo=1,2)","E_LITERAL_KIND"),
        ("(foo=1,bar:2)","E_LITERAL_KIND"),
        ("(foo=1,\"bar\"=2)","E_LITERAL_KIND"),
        ("(\"foo\"=1)","E_LITERAL_KIND"),
        ("(foo=1,,bar=2)","E_LITERAL_KIND"),
        ("(foo=1, (bar=2))","E_LITERAL_KIND"),
    ] {
        rejected(&format!("pub fnc main():int {{let y={expr};return 0;}}"),code);
    }
}

#[test]
fn named_call_arguments_and_record_expansion_stay_distinct_fail_closed() {
    let src="fnc f(int foo):int {return foo;} pub fnc main():int {return f(foo=1);}";
    rejected(src,"E_UNSUPPORTED");
    rejected("fnc f(int foo):int {return foo;} pub fnc main():int {let r=(foo=1);return f(...r);}", "E_UNSUPPORTED");
    let p=parse("fnc f(int foo):int {return foo;} pub fnc main():int {return f((foo=1));}");
    assert!(matches!(&p.funcs[1].body[0],Stmt::Return(Some(e),_)
        if matches!(&e.kind,ExprKind::Call(name,args)
            if name=="f" && args.len()==1 &&
               matches!(&args[0].kind,ExprKind::Record(fields) if fields.len()==1))));
}

#[test]
fn adjacent_generic_close_and_assignment_preserve_type_and_comparison_tokens() {
    // Lexer emits Ge for adjacent '>='; only type parsing is allowed
    // to split it into '>' plus '='.
    let p = binding(
        "let m:Map<string,int>={}; let n:Map<string,List<int>>={}; \
         let a:Array<int>=[]; let comparison=3>=2;"
    );
    let typed = collection_types::analyze(&p).unwrap();
    assert_eq!(typed.len(), 3);
    assert_eq!(typed[0].value.ty, LiteralType::Map(
        Box::new(LiteralType::String), Box::new(LiteralType::Int)));
    assert_eq!(typed[1].value.ty, LiteralType::Map(
        Box::new(LiteralType::String),
        Box::new(LiteralType::List(Box::new(LiteralType::Int)))));
    assert_eq!(typed[2].value.ty,
        LiteralType::Array(Box::new(LiteralType::Int)));
    assert!(matches!(&p.funcs[0].body[3], Stmt::Bind {init,..}
        if matches!(&init.kind, ExprKind::Binary(..))));
    guest("pub fnc main():int {return 3>=2 ? 42 : 0;}");
}

#[test]
fn duplicate_checkers_agree_across_parens_braces_and_constant_maps() {
    for literal in [
        "(foo=1,foo=2)",
        "(café=1,café=2)",
        "{\"foo\" => 1,\"foo\" => 2}",
        "{1 => true,1 => false}",
    ] {
        rejected(
            &format!("pub fnc main():int {{let value={literal};return 0;}}"),
            "E_DUPLICATE",
        );
    }
    for literal in [
        "{foo:1,\"foo\":2}",
        "{\"foo\":1,foo:2}",
        "{foo:1,'foo' => 2}",
    ] {
        rejected(&format!("pub fnc main():int {{let value={literal};return 0;}}"),
                 "E_LITERAL_KIND");
    }
    // Identifier equality is exact and case sensitive: neither runtime nor
    // compiler may normalize or fold distinct names into one slot.
    guest("pub fnc main():int {let r=(Foo=40,foo=2);return r.Foo+r.foo;}");
    guest("pub fnc main():int {let r=(café=40,cafe=2);return r.café+r.cafe;}");
}

#[test]
fn parser_rejects_ambiguous_nested_and_mixed_named_syntax() {
    for literal in [
        "(foo=1,bar)",
        "(foo=1,bar==2)",
        "(foo=1,if=2)",
        "(foo=1,bar:2)",
        "(foo=1,\"bar\"=2)",
        "(foo=1,bar=2,3)",
    ] {
        rejected(
            &format!("pub fnc main():int {{let r={literal};return 0;}}"),
            "E_LITERAL_KIND",
        );
    }
    rejected("pub fnc main():int {let r=(\"\"=42);return 0;}", "E_LITERAL_KIND");
    rejected(
        "pub fnc main():int {let r=(__atto_closed_record_shape_0=42);return 0;}",
        "E_RESERVED",
    );
    rejected(
        "pub fnc main():int {let r=(foo=1,bar=[1,2]);return 0;}",
        "E_UNSUPPORTED",
    );
}

#[test]
fn parenthesized_record_budget_is_bounded_and_fails_before_guest_creation() {
    let many = (0..1025usize)
        .map(|i| format!("field_{i}=1"))
        .collect::<Vec<_>>()
        .join(",");
    rejected(
        &format!("pub fnc main():int {{let r=({many});return 0;}}"),
        "E_SOURCE_LIMIT",
    );
    // Backend's fixed closed-scalar layout is intentionally narrower than
    // the parser budget. It must reject rather than overflow guest storage.
    let nine = (0..9usize)
        .map(|i| format!("field_{i}=1"))
        .collect::<Vec<_>>()
        .join(",");
    rejected(
        &format!("pub fnc main():int {{let r=({nine});return 0;}}"),
        "E_UNSUPPORTED",
    );
    let eight = (0..8usize)
        .map(|i| format!("field_{i}=1"))
        .collect::<Vec<_>>()
        .join(",");
    guest(&format!(
        "pub fnc main():int {{let r=({eight});return r.field_0+41;}}"
    ));
}

#[test]
fn format_grammar_does_not_reinterpret_call_argument_binders() {
    let cases = [
        "fnc f(int a):int {return a;} pub fnc main():int {return f(a=1);}",
        "fnc f(int a):int {return a;} pub fnc main():int {return f(a:1);}",
        "fnc f(int a):int {return a;} pub fnc main():int {return f(...(a=1));}",
        "pub fnc main():int {stdio.println(value=42);return 0;}",
    ];
    for src in cases {
        rejected(src, "E_UNSUPPORTED");
    }
}

#[test]
fn parenthesized_record_compiles_via_existing_owned_scalar_guest_abi() {
    for src in [
        "pub fnc main():int {let y=(foo=\"star\",abc=5);return y.abc;}",
        "pub fnc main():int {let y=(foo=42);return y.foo;}",
        "pub fnc main():int {let a=(foo=40,bar=2);let b={bar:2,foo:40};return a.foo+b.bar;}",
        "pub fnc main():int {let mut r=(foo=1,bar=2);r[\"foo\"]=40;return r.foo+r.bar;}",
        "pub fnc main():int {val mut Record r=(foo=1);r.foo=42;return r[\"foo\"];}",
    ] {
        guest(src);
    }
}

#[test]
fn wrong_context_and_nested_owned_fields_do_not_bypass_guest_types() {
    rejected("pub fnc main():int {let m:Map<string,int>=(foo=1);return 0;}","E_TYPE");
    rejected("pub fnc main():int {let y=(foo=(bar=1));return 0;}","E_UNSUPPORTED");
    rejected("pub fnc main():int {let y=(foo=1,bar=[1,2]);return 0;}","E_UNSUPPORTED");
}
