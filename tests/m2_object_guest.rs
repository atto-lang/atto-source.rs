//! M2.2: primitive inline fields, real default/user constructors and direct
//! instance dispatch. Ownership beyond a local handle is rejected.
use atto_compiler::compile;
fn main(body:&str)->String {format!("pub fnc main(): int {{\n{body}\n}}\n")}
fn good(src:&str) {
 let guest=compile(src).unwrap_or_else(|d|panic!("expected executable guest: {d}"));
 assert!(guest.starts_with(b"\0asm"));
 assert!(guest.len()<=atto_compiler::wasm::MAX_OUTPUT);
}
fn bad(src:&str,code:&str) {
 let e=compile(src).expect_err("bad class program must not emit a guest");
 assert_eq!(e.code,code,"{e} for:\n{src}");
}
const COUNTER:&str=r#"define class Counter as
 let n: int = 0;
 constructor(value: int) { self.n = value; }
 increment() -> int {
   self.n = self.n + 1;
   return self.n;
 }
end
"#;
#[test]
fn supports_constructor_and_direct_method_calls() {
 good(&format!("{COUNTER}{}",main("val c = new Counter(40);\nreturn c.increment() + 1;")));
 good(&format!("{COUNTER}{}",main("val a = new Counter(1);\nval b = new Counter(10);\nreturn a.increment() + b.increment();")));
}
#[test]
fn supports_default_constructors_and_readonly_fields() {
 let c="define class C as final int base: 40; fnc get() -> int { return self.base; } end\n";
 good(&format!("{c}{}",main("val c = new C();\nreturn c.get() + 2;")));
 good(&format!("define class C as end\n{}",main("val c = new C();return 42;")));
}
#[test]
fn fails_closed_for_final_fields_bad_types_and_ctor_mismatches() {
 let c="define class C as final int x: 1; mutate() { self.x = 2; } end\n";
 bad(&format!("{c}{}",main("val c=new C();return 1;")),"E_IMMUTABLE");
 bad(&format!("define class C as let n: int = false; end\n{}",main("return 42;")),"E_TYPE");
 bad(&format!("define class C as let n: int = 40 + 2; end\n{}",main("return 42;")),"E_UNSUPPORTED");
 bad(&format!("{COUNTER}{}",main("val c=new Counter(true);return 42;")),"E_TYPE");
 bad(&format!("{COUNTER}{}",main("val c=new Counter();return 42;")),"E_ARITY");
 bad(&format!("define class C as constructor(): int {{ return 1; }} end\n{}",main("return 0;")),"E_TYPE");
}
#[test]
fn forbids_alias_escape_and_unchecked_external_field_access() {
 bad(&format!("{COUNTER}{}",main("val a=new Counter(1);val b=a;return 42;")),"E_UNSUPPORTED");
 bad(&format!("{COUNTER}{}",main("val a=new Counter(1);let b=new Counter(2);b=a;return 42;")),"E_UNSUPPORTED");
 bad(&format!("{COUNTER}{}",main("val a=new Counter(1);return a.n;")),"E_UNSUPPORTED");
 bad(&format!("{COUNTER}{}",main("val a=new Counter(1);a.n=40;return 42;")),"E_UNSUPPORTED");
 bad(&format!("{COUNTER}{}",main("new Counter(1);return 42;")),"E_UNSUPPORTED");
 bad(&format!("{COUNTER}fnc escape(v: Counter): int {{ return 3; }}\n{}",main("return 42;")),"E_UNSUPPORTED");
 bad(&format!("{COUNTER}{}",main("val x=new Counter(1);x.constructor(2);return 42;")),"E_UNSUPPORTED");
}
#[test]
fn actor_runtime_remains_separate() {
 bad(&format!("define actor Worker as end\n{}",main("val x=new Worker();return 1;")),"E_UNKNOWN_TYPE");
 good(&format!("define actor Worker as static fnc answer(): int {{return 42;}} end\n{}",
   main("return Worker.answer();")));
}
