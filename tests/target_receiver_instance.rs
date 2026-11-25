//! Instance-bound scalar receiver compiler: a distinct, opt-in contract.
//! Ordinary production compilation and stateful receiver codegen stay closed.
use atto_compiler::target_scalar::{
    compile_scalar_receiver_preview,compile_scalar_receiver_instance_preview
};

const SRC:&str=r#"
use namespace workers;
declare actor Worker;
declare actor Other;
mtd (Worker self) double(int n): int { return n * 2; }
mtd (Worker self) add(int n, int other): int { return n + other; }
mtd (Other self) triple(int n): int { return n * 3; }
"#;

#[test]
fn instance_scalar_receiver_wasm_is_deterministic_and_distinct() {
    let one=compile_scalar_receiver_instance_preview(SRC).unwrap();
    assert_eq!(&one[..8],b"\0asm\x01\0\0\0");
    assert_eq!(one,compile_scalar_receiver_instance_preview(SRC).unwrap());
    assert_ne!(one,compile_scalar_receiver_preview(SRC).unwrap());
    assert!(one.len()<8192);
    assert_eq!(atto_compiler::compile(SRC).unwrap_err().code,"E_UNSUPPORTED");
    assert!(one.windows("atto.bind.Worker".len()).any(|w|w==b"atto.bind.Worker"));
    assert!(one.windows("atto.receiver.Other.triple.1".len())
        .any(|w|w==b"atto.receiver.Other.triple.1"));
}

#[test]
fn instance_scalar_receiver_rejects_unverified_ownership_and_state_effects() {
    for source in [
        "declare actor A; mtd (mut self: A) f(int n): int { return n; }",
        "declare actor A; define actor A as let int count=0; end
         mtd (A self) f(int n): int { return n; }",
        "declare actor A; mtd (A self) f(int n): int { return self.other(); }",
        "declare actor A; mtd (A self) f(int n): int { return n / 2; }",
        "declare actor A; mtd (A self) f(int n): int { return n; return n; }",
    ] {
        assert!(compile_scalar_receiver_instance_preview(source).is_err(),
            "unsafe instance receiver admitted: {source}");
    }
}
