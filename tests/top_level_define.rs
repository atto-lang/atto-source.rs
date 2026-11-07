use atto_compiler::compile;

fn main(body:&str)->String {format!("pub fnc main(): int {{\n  {body}\n}}\n")}
fn pass(src:&str) {
    let binary=compile(src).unwrap_or_else(|e|panic!("expected executable guest: {e}"));
    assert!(binary.starts_with(b"\0asm"),"must emit executable WebAssembly");
    assert!(binary.len()<1_048_576,"compiler guest cap");
}
fn rejects(src:&str, code:&str) {
    let error=compile(src).expect_err("unsupported source must never produce a guest");
    assert_eq!(error.code,code,"{src}: {error}");
}
#[test]
fn class_static_method_and_define_visibility_forms() {
    pass(&format!("define class Calculator as\n  static fnc answer(): int {{ return 42; }}\nend\n{}",main("return Calculator.answer();")));
    pass(&format!("define pub class Calculator {{\n  static fnc answer(): int {{ return 42; }}\n}}\n{}",main("return Calculator.answer();")));
    pass(&format!("pub define class Calculator as\n  static fnc answer(): int {{ return 42; }}\nend\n{}",main("return Calculator.answer();")));
}
#[test]
fn actor_declarations_do_not_forge_actor_runtime() {
    pass(&format!("define actor Worker as\n  static fnc answer(): int {{ return 42; }}\nend\n{}",main("return Worker.answer();")));
    pass(&format!("define actor Worker as end\n{}",main("return 42;")));
    rejects(&format!("define actor Worker as\n  receive(x: int) {{ return 0; }}\nend\n{}",main("return 0;")),"E_UNSUPPORTED");
    rejects(&format!("define actor Worker as end\n{}",main("val w = spawn Worker(); return 0;")),"E_UNSUPPORTED");
}
#[test]
fn enums_are_nominal_and_qualified() {
    pass(&format!("define enum Status as Pending, Ready end\nfnc is_ready(s: Status): bool {{ return s eq Status.Ready; }}\n{}",
        main("if is_ready(Status.Ready) then return 42; fi return 0;")));
    rejects(&format!("define enum Status as Pending, Ready end\n{}",main("return Status.Ready + 1;")),"E_TYPE");
    rejects(&format!("define enum Status as Pending, Ready end\n{}",main("return Status.Ready;")),"E_TYPE");
    rejects(&format!("define enum Status as Pending, Ready end\n{}",main("stdio.println(Status.Ready); return 0;")),"E_TYPE");
    rejects(&format!("define enum Status as Pending, Ready end\ndefine enum Other as Pending, Ready end\n{}",
        main("if Status.Ready eq Other.Ready then return 42; fi return 0;")),"E_TYPE");
    rejects(&format!("define enum Status as Pending, Ready end\n{}",main("return Status.Other;")),"E_UNBOUND");
}
#[test]
fn module_struct_trait_and_interface_declarations() {
    pass(&format!("define module Utils as\n fnc answer(): int {{ return 42; }}\nend\n{}",main("return Utils.answer();")));
    pass(&format!("define struct Packet as end\ndefine interface Format as end\ndefine trait Show as end\n{}",main("return 42;")));
    rejects(&format!("define struct Packet as\n val n: int = 1;\nend\n{}",main("return 42;")),"E_UNSUPPORTED");
}
#[test]
fn fail_closed_duplicate_truncated_and_unsupported() {
    rejects(&format!("define enum Color as Red, Red end\n{}",main("return 0;")),"E_DUPLICATE");
    rejects(&format!("define class C as end\ndefine actor C as end\n{}",main("return 0;")),"E_DUPLICATE");
    rejects(&format!("define class C as\n static fnc f(): int {{ return 1; }}\n static fnc f(): int {{ return 2; }}\nend\n{}",
        main("return C.f();")),"E_DUPLICATE");
    rejects("define class C as\n","E_EOF");
    rejects(&format!("define class C as\n fnc get(): int {{ return 42; }}\nend\n{}",main("return 0;")),"E_UNSUPPORTED");
    rejects(&format!("define class C as end\n{}",main("val c = new C(); return 0;")),"E_UNSUPPORTED");
    rejects(&format!("define enum Color as end\n{}",main("return 0;")),"E_SYNTAX");
    rejects("define enum Color as Red end\n","E_ENTRY");
    rejects(&format!("define class Foo as end\n{}",main("return Foo.bar.baz();")),"E_UNSUPPORTED");
}
