// Lexical slice only. This does not claim the compiler typechecks or lowers
// sequence patterns, nor that any Atto guest has been executed.
#![allow(dead_code)]
#[path = "../src/diag.rs"] mod diag;
#[path = "../src/lexer.rs"] mod lexer;

use lexer::{lex, Tok};

fn kinds(source: &str) -> Vec<Tok> {
    lex(source).expect("lexing should succeed").into_iter().map(|t| t.tok).collect()
}

#[test]
fn ellipsis_is_one_token_without_rewriting_one_or_two_dots() {
    assert_eq!(kinds("..."), vec![Tok::Ellipsis, Tok::Eof]);
    assert_eq!(kinds("...."), vec![Tok::Ellipsis, Tok::Dot, Tok::Eof]);
    assert_eq!(kinds(".."), vec![Tok::Dot, Tok::Dot, Tok::Eof]);
    assert_eq!(kinds("."), vec![Tok::Dot, Tok::Eof]);
    assert_eq!(kinds("x.y"), vec![
        Tok::Ident("x".into()), Tok::Dot, Tok::Ident("y".into()), Tok::Eof
    ]);
}

#[test]
fn counted_pattern_preserves_tokens_and_positions() {
    let got = lex("...(3 of int, 3 of bool)").unwrap();
    assert_eq!(got.iter().map(|x| &x.tok).cloned().collect::<Vec<_>>(), vec![
        Tok::Ellipsis, Tok::LParen, Tok::Int(Some(3)), Tok::Ident("of".into()),
        Tok::Ident("int".into()), Tok::Comma, Tok::Int(Some(3)),
        Tok::Ident("of".into()), Tok::Ident("bool".into()), Tok::RParen, Tok::Eof,
    ]);
    assert_eq!(got[0].pos, diag::Pos {line:1,col:1});
    assert_eq!(got[1].pos, diag::Pos {line:1,col:4});
    assert_eq!(got[2].pos, diag::Pos {line:1,col:5});
}

#[test]
fn legacy_spelling_is_not_an_implicit_alias() {
    assert_eq!(kinds("..._"), vec![
        Tok::Ellipsis, Tok::Ident("_".into()), Tok::Eof
    ]);
    assert_eq!(kinds("_..."), vec![
        Tok::Ident("_".into()), Tok::Ellipsis, Tok::Eof
    ]);
    // Grammar must reject these later. The lexer never merges the spellings.
}

#[test]
fn numbers_adjacent_to_ellipsis_are_not_decimal_floats() {
    assert_eq!(kinds("1...2"), vec![
        Tok::Int(Some(1)), Tok::Ellipsis, Tok::Int(Some(2)), Tok::Eof
    ]);
    assert_eq!(kinds("1.5"), vec![Tok::Float(1.5_f64.to_bits()), Tok::Eof]);
}

#[test]
fn multi_line_ellipsis_positions_advance_by_three_columns() {
    let got=lex("x...\n...y").unwrap();
    assert_eq!(got[1].tok,Tok::Ellipsis);
    assert_eq!(got[1].pos,diag::Pos{line:1,col:2});
    assert_eq!(got[2].tok,Tok::Ellipsis);
    assert_eq!(got[2].pos,diag::Pos{line:2,col:1});
    assert_eq!(got[3].pos,diag::Pos{line:2,col:4});
}
