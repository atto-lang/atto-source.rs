//! Source-owned Atto core modules and SOURCE-FREE reference oracle.
//! This does not call attoc, link a guest, or certify import parity.
const MATH: &str = include_str!("../library/atto.core/math.atto");
const HASH: &str = include_str!("../library/atto.core/hash.atto");
const CELL: &str = include_str!("../library/atto.core/cell.atto");
const MOD: i64 = 1_000_000_007;
fn step(s: i64, byte: i64) -> i64 {
    if !(0..MOD).contains(&s) || !(0..=255).contains(&byte) { return -1; }
    let mut v = s * 31 + byte;
    for _ in 0..31 {
        if v >= MOD { v -= MOD; }
    }
    v
}
#[test]
fn source_tree_has_three_native_atto_modules_without_host_fallback() {
    for (name, contents) in [("math", MATH), ("hash", HASH), ("cell", CELL)] {
        assert!(contents.starts_with("//"));
        assert!(contents.contains(&format!("define module {name} as")));
        assert!(!contents.contains("java:"));
        assert!(!contents.contains("dotnet:"));
        assert!(!contents.contains("import module std"));
        assert!(contents.len() <= 1_048_576);
    }
    assert!(MATH.contains("contains_closed_int"));
    assert!(HASH.contains("step_byte"));
    assert!(CELL.contains("not Cell<T>") || CELL.contains("not Cell<T> or RefCell<T>"));
}
#[test]
fn checked_byte_fold_reference_vectors() {
    for s in [0,1,17,569,MOD-2,MOD-1] {
        for byte in 0..=255 {
            assert_eq!(step(s,byte),(s*31+byte)%MOD);
        }
    }
    assert_eq!(step(step(17,42),7),17_646);
    assert_eq!(step(-1,1),-1);
    assert_eq!(step(17,256),-1);
}
#[test]
fn borrowing_admission_is_policy_not_a_granted_loan() {
    let read=|readers:i64,writer:bool|readers>=0&&readers<=65_535&&!writer;
    let write=|readers:i64,writer:bool|readers==0&&!writer;
    assert!(read(65_535,false));
    assert!(!read(65_536,false));
    assert!(!read(3,true));
    assert!(write(0,false));
    assert!(!write(1,false));
    assert!(!write(0,true));
    assert!(CELL.contains("not borrow tokens"));
}
