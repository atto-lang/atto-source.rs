//! Production checker / Wasm emission, not standalone graph model tests.
use atto_compiler::compile;

const ROOT: &str = "define class FixedLeaf as pub val int n = 20; end define class Parcel as pub let FixedLeaf child = new FixedLeaf(); pub let int extra = 0; end ";

#[test]
fn nested_immutable_root_emits_deterministic_wasm() {
    let source = include_str!("fixtures/pass/copy-immutable-nested-root.atto");
    let bytes = compile(source).unwrap_or_else(|d| panic!("{d}"));
    assert_eq!(&bytes[..8], b"\0asm\x01\0\0\0");
    assert_eq!(compile(source).unwrap(), bytes);
}

#[test]
fn nested_leaf_never_becomes_an_owned_projection() {
    for body in [
        "let a = new Parcel(); let b = copy! a; let escaped = b.child; return escaped.n;",
        "let a = new Parcel(); let b = copy! a; let escaped = copy!! b.child; return escaped.n;",
        "let a = new Parcel(); let b = copy! a; let x = new FixedLeaf(); b.child = x; return a.child.n;",
    ] {
        let source = format!("{ROOT}pub fnc main(): int {{ {body} }}");
        assert_eq!(compile(&source).unwrap_err().code, "E_BORROW", "{source}");
    }
}

#[test]
fn no_mutable_leaf_or_constructor_method_witness() {
    let main = "pub fnc main(): int { let a = new Parcel(); let b = copy! a; return b.child.n; }";
    for root in [
        ROOT.replace("pub val int n", "pub let int n"),
        ROOT.replace("pub let FixedLeaf child", "pub let mut FixedLeaf child"),
        ROOT.replace("pub val int n = 20; end", "pub val int n = 20; read(): int { return self.n; } end"),
    ] {
        let src = format!("{root}{main}");
        assert_eq!(compile(&src).unwrap_err().code, "E_UNSUPPORTED_TYPE", "{src}");
    }
}

#[test]
fn fixed_outer_slot_and_nested_writes_are_denied() {
    let fixed = ROOT.replace("pub let FixedLeaf child", "pub val FixedLeaf child");
    let source = format!("{fixed}pub fnc main(): int {{ let a = new Parcel(); let b = copy! a; b.child = new FixedLeaf(); return b.child.n; }}");
    assert_eq!(compile(&source).unwrap_err().code, "E_IMMUTABLE");
    let source = format!("{ROOT}pub fnc main(): int {{ let a = new Parcel(); let b = copy! a; a.child.n = 22; return b.child.n; }}");
    assert_eq!(compile(&source).unwrap_err().code, "E_OWNERSHIP");
}

#[test]
fn scalar_projection_does_not_make_reference_extraction_legal() {
    let positive = format!("{ROOT}pub fnc main(): int {{ let a = new Parcel(); let b = copy! a; return a.child.n + b.child.n + 2; }}");
    assert!(compile(&positive).is_ok());
    for body in [
        "let a = new Parcel(); let b = copy! a; let child = a.child; return child.n;",
        "let a = new Parcel(); let b = copy! a; let child = b.child; return child.n;",
        "let a = new Parcel(); let b = copy! a; let child = copy!! b.child; return child.n;",
    ] {
        let source = format!("{ROOT}pub fnc main(): int {{ {body} }}");
        assert_eq!(compile(&source).unwrap_err().code, "E_BORROW", "{source}");
    }
}
