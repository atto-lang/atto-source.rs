use atto_compiler::compile;

const POSITIVE:&str=r#"
pub fnc main(): int {
    const Record<infer T1> r=(a=true,b="xyz",c=5);
    const Tuple<infer T2> t=(true,"xyz",5);
    const struct<infer T3> s={a:true,b:"xyz",c:5};
    const Record<T1> r2=(c=10,b="abc",a=false);
    const Tuple<T2> t2=(false,"abc",12);
    const struct<T3> s2={c:20,b:"abc",a:false};
    return r2.c+t2[2]+s2.c;
}
"#;

#[test]
fn named_scalar_aggregate_reuse_emits_checked_guest() {
    let bytes=compile(POSITIVE).expect("named aggregate scalar guest");
    assert!(bytes.starts_with(b"\0asm\x01\0\0\0"));
    assert_eq!(bytes,compile(POSITIVE).unwrap());
}
#[test]
fn malformed_or_mismatched_shapes_emit_no_guest() {
    for (body,code) in [
      ("const Record<infer> r=(a=1);","E_TYPE_NAME"),
      ("const Record<infer T> r=(a=1); const Record<U> s=(a=1);","E_UNKNOWN_TYPE"),
      ("const Record<infer T> r=(a=1); const Tuple<infer T> s=(1,2);","E_DUPLICATE_TYPE"),
      ("const Record<infer T> r=(a=1,a=2);","E_DUPLICATE_FIELD"),
      ("const Record<infer T> r=(a=1); const Record<T> s=(a=true);","E_TYPE_MISMATCH"),
      ("const Record<infer T> r=(a=1); const Record<T> s=(a=1,b=2);","E_TYPE_MISMATCH"),
      ("const Record<infer T> r=(a=1,b=2); const Record<T> s=(a=1);","E_TYPE_MISMATCH"),
      ("const Tuple<infer T> r=(1,true); const Tuple<T> s=(true,1);","E_TYPE_MISMATCH"),
      ("const Tuple<infer T> r=(1,true); const Tuple<T> s=(1);","E_TYPE_MISMATCH"),
      ("const struct<infer T> r={a:1}; const Record<T> s=(a=1);","E_AGGREGATE_KIND"),
      ("const Record<infer T> r=(a=unknown);","E_TYPE_INFER_AMBIGUOUS"),
    ] {
        let source=format!("pub fnc main(): int {{ {body} return 0; }}");
        assert_eq!(compile(&source).unwrap_err().code,code,"{source}");
    }
}
#[test]
fn no_whole_aggregate_or_runtime_identity_escape() {
    for result in ["r","r.missing","r.a + unknown"] {
        let source=format!("pub fnc main(): int {{ const Record<infer T> r=(a=1); return {result}; }}");
        assert!(compile(&source).is_err(),"{source}");
    }
}
