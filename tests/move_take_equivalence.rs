//! Both transfer spellings must use the same checked lowering, not merely parse.
use atto_compiler::compile;

const OWNER: &str = include_str!("fixtures/pass/move-bang-owned-class.atto");
const MOVED: &str = include_str!("fixtures/fail/move-bang-use-after-transfer.atto");
const LOAN: &str = include_str!("fixtures/fail/move-bang-active-loan.atto");

#[test]
fn transfer_spellings_emit_identical_wasm() {
    let moved = compile(OWNER).expect("move! must compile");
    let taken = compile(&OWNER.replace("move! ", "take! ")).expect("take! must compile");
    assert_eq!(&moved[..8], b"\0asm\x01\0\0\0");
    assert_eq!(moved, taken);
}

#[test]
fn both_spellings_preserve_move_and_live_loan_diagnostics() {
    for (source, code) in [(MOVED, "E_MOVED"), (LOAN, "E_BORROW")] {
        for candidate in [source.to_string(), source.replace("move! ", "take! ")] {
            let denied = compile(&candidate).expect_err("invalid ownership transfer");
            assert_eq!(denied.code, code, "{denied}");
        }
    }
}

#[test]
fn bare_move_is_not_a_reserved_identifier() {
    compile(include_str!("fixtures/pass/move-bare-identifier.atto")).unwrap();
}

#[test]
fn compact_move_is_not_admitted() {
    let denied = compile(include_str!("fixtures/fail/move-bang-missing-separator.atto"))
        .expect_err("missing separator");
    assert_eq!(denied.code, "E_BANG_KEYWORD_WHITESPACE");
}
