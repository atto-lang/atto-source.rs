//! Scalar-only executable candidate for StructName(field=literal) on Rust/Wasm.
//! Tests compile through the real compiler entrypoint. Runtime execution is
//! independently checked by tests/struct-equals-guest.mjs, not these unit tests.
use atto_compiler::compile;

const HEADER: &str = "define struct Particle as int x; int y; end\n";
fn source(body: &str) -> String {
    format!("{HEADER}pub fnc main(): int {{ {body} }}")
}
fn pass(program: &str) {
    let wasm = compile(program).unwrap_or_else(|e| panic!("expected guest Wasm, got {e}"));
    assert!(wasm.starts_with(b"\0asm\x01\0\0\0"));
    assert_eq!(wasm, compile(program).unwrap(), "deterministic guest bytes");
}
fn reject(program: &str, code: &str) {
    let err = compile(program).expect_err("invalid initializer must not emit Wasm");
    assert_eq!(err.code, code, "{err}");
    assert!(err.pos.line > 0 && err.pos.col > 0, "{err}");
}

#[test]
fn implicit_struct_equals_and_brace_forms_compile() {
    pass(&source(
        "val a = Particle{x: 10, y: 20};\n\
         val b = Particle(x=10, y=20);\n\
         return a.x + b.y;"
    ));
    pass(&source("val p = Particle(y=20, x=10); return p.x + p.y;"));
    pass(&source("val p = Particle(x=10,y=20,); return p.x + p.y;"));
    pass(&source("val p = Particle(x=10,y=20); return p.y;"));
}

#[test]
fn missing_duplicate_unknown_and_mixed_fields_fail_closed() {
    reject(&source("val p=Particle(x=10); return p.x;"), "E_SHAPE");
    reject(&source("val p=Particle(x=10,x=11,y=20); return p.x;"), "E_DUPLICATE");
    reject(&source("val p=Particle(x=10,y=20,velocity=1); return p.x;"), "E_NO_FIELD");
    reject(&source("val p=Particle(x=10,y:20); return p.x;"), "E_STRUCT_INIT");
}

#[test]
fn calls_ctors_shadowing_and_effectful_values_never_reinterpret_equals() {
    reject(&source("val Particle=7; val p=Particle(x=10,y=20); return p.x;"), "E_STRUCT_RESOLVE");
    reject(&format!("{HEADER}fnc Particle(int x) -> int {{ return x; }}\n\
        pub fnc main(): int {{ val p=Particle(x=10,y=20); return p.x; }}"), "E_STRUCT_RESOLVE");
    reject(&source("val p=new Particle(x=10,y=20); return p.x;"), "E_STRUCT_TARGET");
    reject(&source("val p=Particle(x=increment(10),y=20); return p.x;"), "E_UNSUPPORTED");
    // An ordinary function named 'foo' is NEVER a struct initializer.
    reject("fnc foo(int x) -> int { return x; }\n\
        pub fnc main(): int { return foo(x=41); }", "E_SYNTAX");
}

#[test]
fn explicit_class_constructor_is_not_an_equals_struct_initializer() {
    let source = "define class Box as\n\
          int value: 0;\n\
          constructor(int value) { self.value = value; }\n\
        end\n\
        pub fnc main(): int { val b = new Box(value=42); return b.value; }";
    assert!(compile(source).is_err(), "equals labels cannot reach class constructor ABI");
}
