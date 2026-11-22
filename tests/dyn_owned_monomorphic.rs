//! The first browser Dyn<I> slice is closed-world monomorphic only.
//! These are compiler/guest-byte tests, not proof of a polymorphic vtable ABI.
use atto_compiler::compile;
const EXIT: &str = "stdio.println(display(erased));\n  return 0;";

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
fn checked_as_owned_erasure_compiles_with_the_same_guest_bytes() {
    let canonical = compile(POSITIVE).expect("constructor form");
    for rhs in [
        "(take! concrete) as Dyn<Printable>",
        "take! concrete as Dyn<Printable>",
    ] {
        let src = POSITIVE.replace("Dyn<Printable>(take! concrete)", rhs);
        let wasm = compile(&src).unwrap_or_else(|e| panic!("{rhs}: {e}"));
        assert!(wasm.starts_with(&[0, 97, 115, 109, 1, 0, 0, 0]));
        assert_eq!(wasm, canonical, "{rhs}: owned erasure must be equivalent");
    }
}

#[test]
fn checked_as_requires_matching_typed_target_and_an_owned_named_source() {
    for rhs in [
        "(borrow! concrete) as Dyn<Printable>",
        "(concrete) as Dyn<Printable>",
        "(take! new Greeting()) as Dyn<Printable>",
        "(take! concrete) as Dyn<Other>",
        "(take! concrete) as Dyn<Printable> as Dyn<Printable>",
        "42 as Dyn<Printable>",
    ] {
        let src = POSITIVE.replace("Dyn<Printable>(take! concrete)", rhs);
        // An unknown Dyn interface may diagnose before checking the cast
        // pattern; both errors must fail without publishing a guest.
        let diagnostic = compile(&src).expect_err("unsafe as Dyn<I> must fail");
        assert!(
            diagnostic.code == "E_DYN_OWNERSHIP" || diagnostic.code == "E_DYN_INTERFACE",
            "{rhs}: {diagnostic}"
        );
    }
    error(&POSITIVE.replace(
        "val Dyn<Printable> erased = Dyn<Printable>(take! concrete);",
        "val Greeting erased = (take! concrete) as Dyn<Printable>;",
    ), "E_DYN_OWNERSHIP");
    error(&POSITIVE.replace(
        "val Dyn<Printable> erased = Dyn<Printable>(take! concrete);",
        "val erased = (take! concrete) as Dyn<Printable>;",
    ), "E_DYN_OWNERSHIP");
}

#[test]
fn checked_as_preserves_use_after_move_and_interface_visibility() {
    let cast = POSITIVE.replace(
        "Dyn<Printable>(take! concrete)", "(take! concrete) as Dyn<Printable>");
    error(&cast.replace(EXIT,
        "stdio.println(concrete.describe()); return 0;"), "E_MOVED");
    error(&cast.replace(EXIT,
        "stdio.println(erased.secret()); return 0;"), "E_DYN_INTERFACE");
}

#[test]
fn moved_source_owner_is_unusable() {
    let src = POSITIVE.replace(
        EXIT,
        "stdio.println(concrete.describe()); return 0;");
    error(&src, "E_MOVED");
}

#[test]
fn no_interface_forgery_or_concrete_member_leak() {
    error(&POSITIVE.replace("Dyn<Printable>(take! concrete)", "Dyn<Printable>(42)"),
        "E_DYN_OWNERSHIP");
    error(&POSITIVE.replace(EXIT, "stdio.println(erased.secret()); return 0;"),
        "E_DYN_INTERFACE");
    error(&POSITIVE.replace(EXIT, "return erased.secret;"),
        "E_DYN_INTERFACE");
    error(&POSITIVE.replace(EXIT,
        "val Greeting leaked = erased; return leaked.secret();"), "E_DYN_INTERFACE");
    error(&POSITIVE.replace(EXIT,
        "return send_to_other_tenant(erased);"), "E_DYN_INTERFACE");
}

#[test]
fn no_implicit_dyn_conversion_at_call_sites() {
    let src = POSITIVE.replace(
        EXIT, "stdio.println(display(concrete)); return 0;");
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
fn interface_methods_accept_checked_scalar_parameters() {
    let src = POSITIVE
        .replace("describe(): string;", "describe(int code): string;")
        .replace("pub describe(): string", "pub describe(int code): string")
        .replace("item.describe();", "item.describe(1);");
    let wasm = compile(&src).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(&wasm[..8], b"\0asm\x01\0\0\0");
}

#[test]
fn object_and_handle_method_signatures_fail_before_type_erasure() {
    for signature in [
        "describe(): Greeting;",
        "describe(): Self;",
        "describe(ActorRef<Worker> peer): string;",
        "describe(Pointer<Greeting> borrowed): string;",
        "describe(Greeting object): string;",
        "describe(Any hidden): string;",
        "describe(): Option<Greeting>;",
    ] {
        let src = POSITIVE.replace("describe(): string;", signature);
        error(&src, "E_DYN_ABI");
    }
}

#[test]
fn unnamed_or_borrowed_erasure_is_not_silently_lowered() {
    error(&POSITIVE.replace("val Dyn<Printable> erased = Dyn<Printable>(take! concrete);",
        "val Greeting erased = Dyn<Printable>(take! concrete);"), "E_DYN_OWNERSHIP");
    error(&POSITIVE.replace("Dyn<Printable>(take! concrete)",
        "Dyn<Printable>(borrow! concrete)"), "E_DYN_OWNERSHIP");
    error(&POSITIVE.replace("Dyn<Printable>(take! concrete)",
        "Dyn<Printable>(take! new Greeting())"), "E_DYN_OWNERSHIP");
    error(&POSITIVE.replace("val Dyn<Printable> erased = Dyn<Printable>(take! concrete);",
        "val Dyn<Printable> erased = concrete;"), "E_DYN_OWNERSHIP");
}
