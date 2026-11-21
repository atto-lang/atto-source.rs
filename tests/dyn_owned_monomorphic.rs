//! The first browser Dyn<I> slice is closed-world monomorphic only.
//! These are compiler/guest-byte tests, not proof of a polymorphic vtable ABI.
use atto_compiler::compile;

const POSITIVE: &str = r#"
define pub interface Printable as
  describe(): string;
end

define class Greeting implements Printable as
  pub describe(): string { return "hello"; }
  pub secret(): int { return 17; }
end

fnc display(Dyn<Printable> item): string {
  return item.describe();
}

pub fnc main(): int {
  val Greeting concrete = new Greeting();
  val Dyn<Printable> erased = Dyn<Printable>(take! concrete);
  stdio.println(display(erased));
  return 0;
}
"#;

fn error(src: &str, code: &str) {
    let err = compile(src).expect_err("Dyn error must not produce guest Wasm");
    assert_eq!(err.code, code, "{err}");
    assert!(err.pos.line > 0 && err.pos.col > 0, "{err}");
}

#[test]
fn single_verified_implementor_compiles_reproducibly() {
    let wasm = compile(POSITIVE).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(&wasm[..8], b"\0asm\x01\0\0\0");
    assert_eq!(wasm, compile(POSITIVE).unwrap());
}

#[test]
fn inferred_owned_erasure_is_also_accepted() {
    let src = POSITIVE.replace(
        "Dyn<Printable>(take! concrete)", "take! concrete");
    let wasm = compile(&src).expect("the target Dyn type should infer erasure");
    assert_eq!(&wasm[..8], b"\0asm\x01\0\0\0");
}

#[test]
fn moved_source_owner_is_unusable() {
    let src = POSITIVE.replace(
        "stdio.println(display(erased));
  return 0;",
        "stdio.println(concrete.describe()); return 0;");
    error(&src, "E_MOVED");
}

#[test]
fn no_interface_forgery_or_concrete_member_leak() {
    error(&POSITIVE.replace("Dyn<Printable>(take! concrete)", "Dyn<Printable>(42)"),
        "E_DYN_OWNERSHIP");
    error(&POSITIVE.replace("stdio.println(display(erased));
  return 0;", "stdio.println(erased.secret()); return 0;"),
        "E_DYN_INTERFACE");
    error(&POSITIVE.replace("stdio.println(display(erased));
  return 0;", "return erased.secret;"),
        "E_DYN_INTERFACE");
    error(&POSITIVE.replace("stdio.println(display(erased));
  return 0;",
        "val Greeting leaked = erased; return leaked.secret();"), "E_DYN_INTERFACE");
    error(&POSITIVE.replace("stdio.println(display(erased));
  return 0;",
        "return send_to_other_tenant(erased);"), "E_DYN_INTERFACE");
}

#[test]
fn no_implicit_dyn_conversion_at_call_sites() {
    let src = POSITIVE.replace(
        "stdio.println(display(erased));
  return 0;", "stdio.println(display(concrete)); return 0;");
    // This is refused at the interface call-site regardless of the move.
    error(&src, "E_DYN_INTERFACE");
}

#[test]
fn unverified_or_polymorphic_runtime_dispatch_fails_closed() {
    let src = POSITIVE.replace("implements Printable", "");
    error(&src, "E_DYN_RUNTIME");
    let extra = r#"
define class Farewell implements Printable as
    pub describe(): string { return "goodbye"; }
end
"#;
    let src = POSITIVE.replace("fnc display", &format!("{extra}\nfnc display"));
    error(&src, "E_DYN_RUNTIME");
    error(&POSITIVE.replace("Dyn<Printable>", "Dyn<Missing>"),
        "E_DYN_INTERFACE");
}

#[test]
fn unnamed_or_borrowed_erasure_is_not_silently_lowered() {
    error(&POSITIVE.replace("Dyn<Printable>(take! concrete)",
        "Dyn<Printable>(borrow! concrete)"), "E_DYN_OWNERSHIP");
    error(&POSITIVE.replace("Dyn<Printable>(take! concrete)",
        "Dyn<Printable>(take! new Greeting())"), "E_DYN_OWNERSHIP");
    error(&POSITIVE.replace("val Dyn<Printable> erased = Dyn<Printable>(take! concrete);",
        "val Dyn<Printable> erased = concrete;"), "E_DYN_OWNERSHIP");
}
