//! G1 structural admission for Atto's proposed `unsafe { ... }` boundary.
//!
//! This parser builds a bounded source-span AST for lexical unsafe blocks and
//! direct unsafe callable declarations. It never emits guest code, grants
//! capabilities, or substitutes for the production nominal/type/effect checker.
//! Browser/Wasm guests still reject *all* unsafe source before any lowerer.
//! It runs on real lexed Atto tokens, so quoted text and comments are masked.

use std::collections::BTreeMap;

use crate::diag::{Diagnostic, Pos};
use crate::lexer::{Tok, Token};

const MAX_UNSAFE_BLOCKS: usize = 128;
const MAX_UNSAFE_CALLABLES: usize = 128;
const MAX_DELIMITER_DEPTH: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnsafeBlock {
    pub open: usize,
    pub close: usize,
    pub pos: Pos,
    pub parent: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnsafeCallable {
    pub name: String,
    pub pos: Pos,
    pub name_token: usize,
    pub body_open: usize,
    pub body_close: usize,
    /// The callable body is NOT implicitly an unsafe context.
    pub requires_unsafe: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnsafeCall {
    pub name: String,
    pub pos: Pos,
    pub token_index: usize,
    pub unsafe_block: Option<usize>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnsafeSyntaxTree {
    pub blocks: Vec<UnsafeBlock>,
    pub callables: Vec<UnsafeCallable>,
    pub direct_calls: Vec<UnsafeCall>,
    pub first_unsafe: Option<Pos>,
}

fn word(token: &Token, target: &str) -> bool {
    matches!(&token.tok, Tok::Ident(s) if s == target)
}

fn ident(token: &Token) -> Option<&str> {
    match &token.tok {
        Tok::Ident(s) => Some(s),
        _ => None,
    }
}

fn open_delimiter(tok: &Tok) -> bool {
    matches!(tok, Tok::LBrace | Tok::LParen | Tok::LBracket)
}

fn matching(open: &Tok, close: &Tok) -> bool {
    matches!((open, close),
        (Tok::LBrace, Tok::RBrace)
        | (Tok::LParen, Tok::RParen)
        | (Tok::LBracket, Tok::RBracket))
}

/// Structural delimiter map for the checked source. Unlike lexical
/// substring search, this respects strings, nested comments and brace types.
fn delimiters(tokens: &[Token]) -> Result<Vec<Option<usize>>, Diagnostic> {
    let mut matches = vec![None; tokens.len()];
    let mut stack: Vec<usize> = Vec::new();
    for (i, t) in tokens.iter().enumerate() {
        if open_delimiter(&t.tok) {
            if stack.len() >= MAX_DELIMITER_DEPTH {
                return Err(Diagnostic::new(t.pos, "E_SOURCE_LIMIT",
                    "unsafe analysis exceeded delimiter nesting budget"));
            }
            stack.push(i);
        } else if matches!(t.tok, Tok::RBrace | Tok::RParen | Tok::RBracket) {
            let Some(open) = stack.pop() else {
                return Err(Diagnostic::new(t.pos, "E_UNSAFE_SYNTAX",
                    "unmatched closing delimiter in unsafe source"));
            };
            if !matching(&tokens[open].tok, &t.tok) {
                return Err(Diagnostic::new(t.pos, "E_UNSAFE_SYNTAX",
                    "mismatched delimiter in unsafe source"));
            }
            matches[i] = Some(open);
            matches[open] = Some(i);
        }
    }
    if let Some(open) = stack.pop() {
        return Err(Diagnostic::new(tokens[open].pos, "E_UNSAFE_SYNTAX",
            "unclosed delimiter in unsafe source"));
    }
    Ok(matches)
}

/// Identify brace-terminated callable declarations without trying to infer
/// effects or grant authority. The G1 profile only admits direct named fnc /
/// routine declarations with ordinary `{ ... }` bodies.
fn callable_body(
    tokens: &[Token], pair: &[Option<usize>], name_at: usize,
) -> Result<(usize, usize), Diagnostic> {
    if !matches!(tokens.get(name_at + 1).map(|t| &t.tok), Some(Tok::LParen)) {
        return Err(Diagnostic::new(tokens[name_at].pos, "E_UNSAFE_SYNTAX",
            "unsafe callable requires a named parameter list"));
    }
    let params_end = pair[name_at + 1].ok_or_else(|| Diagnostic::new(
        tokens[name_at].pos, "E_UNSAFE_SYNTAX", "unterminated unsafe callable parameters"))?;
    // Only a braced body is in the current G1 surface. The signature may
    // include : Return, -> Return, generic type brackets or effect clauses;
    // do not mistake a brace inside a parameter list for the body.
    for at in params_end + 1..tokens.len() {
        match tokens[at].tok {
            Tok::LBrace => {
                let end = pair[at].ok_or_else(|| Diagnostic::new(tokens[at].pos,
                    "E_UNSAFE_SYNTAX", "missing unsafe callable body close"))?;
                return Ok((at, end));
            }
            Tok::Semi | Tok::Eof | Tok::RBrace => break,
            _ => {}
        }
    }
    Err(Diagnostic::new(tokens[name_at].pos, "E_UNSAFE_SYNTAX",
        "unsafe callable G1 syntax requires a braced body"))
}

/// Build a structural, *nonexecutable* unsafe syntax tree and validate the
/// narrow lexical obligation for direct locally declared unsafe calls.
pub fn analyze(tokens: &[Token]) -> Result<UnsafeSyntaxTree, Diagnostic> {
    let unsafe_at: Vec<usize> = tokens.iter().enumerate()
        .filter_map(|(i, t)| word(t, "unsafe").then_some(i))
        .collect();
    if unsafe_at.is_empty() {
        return Ok(UnsafeSyntaxTree::default());
    }
    let pair = delimiters(tokens)?;
    let mut out = UnsafeSyntaxTree {
        first_unsafe: Some(tokens[unsafe_at[0]].pos),
        ..UnsafeSyntaxTree::default()
    };
    for at in unsafe_at {
        let Some(next) = tokens.get(at + 1) else {
            return Err(Diagnostic::new(tokens[at].pos, "E_UNSAFE_SYNTAX",
                "unsafe requires a block or an unsafe fnc declaration"));
        };
        match &next.tok {
            Tok::LBrace => {
                if out.blocks.len() >= MAX_UNSAFE_BLOCKS {
                    return Err(Diagnostic::new(tokens[at].pos, "E_SOURCE_LIMIT",
                        "too many unsafe blocks"));
                }
                let close = pair[at + 1].ok_or_else(|| Diagnostic::new(
                    next.pos, "E_UNSAFE_SYNTAX", "unterminated unsafe block"))?;
                let parent = out.blocks.iter().enumerate()
                    .filter(|(_, b)| b.open < at && at < b.close)
                    .max_by_key(|(_, b)| b.open)
                    .map(|(i, _)| i);
                out.blocks.push(UnsafeBlock {
                    open: at + 1, close, pos: tokens[at].pos, parent,
                });
            }
            Tok::Ident(kind) if kind == "fnc" || kind == "routine" => {
                if out.callables.len() >= MAX_UNSAFE_CALLABLES {
                    return Err(Diagnostic::new(tokens[at].pos, "E_SOURCE_LIMIT",
                        "too many unsafe callable declarations"));
                }
                // Non-top-level unsafe declarations are deliberately withheld
                // pending method/closure effect and receiver resolution.
                let in_brace = pair.iter().enumerate().any(|(open, close)|
                    open < at && matches!(tokens[open].tok, Tok::LBrace)
                    && close.is_some_and(|end| end > at));
                if in_brace {
                    return Err(Diagnostic::new(tokens[at].pos, "E_UNSAFE_CALL",
                        "nested unsafe declarations need typed receiver and closure-effect checking"));
                }
                let name_at = at + 2;
                let Some(name) = tokens.get(name_at).and_then(ident) else {
                    return Err(Diagnostic::new(tokens[at].pos, "E_UNSAFE_SYNTAX",
                        "unsafe fnc requires a named callable"));
                };
                let (body_open, body_close) = callable_body(tokens, &pair, name_at)?;
                if out.callables.iter().any(|f| f.name == name) {
                    return Err(Diagnostic::new(tokens[at].pos, "E_UNSAFE_CALL",
                        "duplicate unsafe callable name"));
                }
                out.callables.push(UnsafeCallable {
                    name: name.to_owned(), pos: tokens[at].pos, name_token: name_at,
                    body_open, body_close, requires_unsafe: true,
                });
            }
            _ => return Err(Diagnostic::new(tokens[at].pos, "E_UNSAFE_SYNTAX",
                "unsafe must precede a braced block or a named fnc/routine declaration")),
        }
    }

    // A direct unsafe call must be inside a lexically enclosing unsafe block.
    // Signatures, method calls and dynamically dispatched/inlined calls need
    // the production typed checker; this G1 AST never claims they are safe.
    let decl_names: BTreeMap<&str, usize> = out.callables.iter()
        .map(|f| (f.name.as_str(), f.name_token)).collect();
    for (i, token) in tokens.iter().enumerate() {
        let Some(name) = ident(token) else { continue };
        if !decl_names.contains_key(name) || decl_names.get(name) == Some(&i) {
            continue;
        }
        if !matches!(tokens.get(i + 1).map(|t| &t.tok), Some(Tok::LParen)) {
            continue;
        }
        if i > 0 && matches!(tokens[i - 1].tok, Tok::Dot | Tok::ColonColon) {
            continue; // method/qualified-call name is not a proven local fnc
        }
        // Safe functions with the same name require proper nominal resolution;
        // do not accidentally treat their header as a call.
        if i > 0 && (word(&tokens[i - 1], "fnc") || word(&tokens[i - 1], "routine")) {
            continue;
        }
        let owner = out.blocks.iter().enumerate()
            .filter(|(_, b)| b.open < i && i < b.close)
            .max_by_key(|(_, b)| b.open)
            .map(|(j, _)| j);
        if owner.is_none() {
            return Err(Diagnostic::new(token.pos, "E_UNSAFE_CONTEXT",
                format!("direct call to unsafe fnc '{name}' requires unsafe {{ ... }}")));
        }
        // In v1 a nested callable must not inherit the outer unsafe block.
        if tokens.iter().enumerate().any(|(j, t)| j < i
            && out.blocks[owner.unwrap()].open < j
            && (word(t, "fnc") || word(t, "routine"))) {
            return Err(Diagnostic::new(token.pos, "E_UNSAFE_CALL",
                "unsafe context cannot be inherited by nested function declarations"));
        }
        out.direct_calls.push(UnsafeCall {
            name: name.to_owned(), pos: token.pos, token_index: i,
            unsafe_block: owner,
        });
    }
    Ok(out)
}

/// The browser compiler is a guest compiler with no trusted native-host grant.
/// Parsing/structural checking cannot turn it into a native FFI executor.
pub fn reject_unlowered(tokens: &[Token]) -> Result<(), Diagnostic> {
    let parsed = analyze(tokens)?;
    if let Some(pos) = parsed.first_unsafe {
        return Err(Diagnostic::new(pos, "E_UNSAFE_DOMAIN",
            "unsafe blocks and unsafe callables are forbidden in browser/Wasm guests; lexical syntax does not grant native host authority"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{compile, lexer};

    fn parse(source: &str) -> Result<UnsafeSyntaxTree, Diagnostic> {
        analyze(&lexer::lex(source).unwrap())
    }

    #[test]
    fn parses_nested_blocks_and_source_spans_without_guest_execution() {
        let ast = parse("pub fnc main(): int {\n  unsafe { unsafe { val x = 42; } }\n  return 42;\n}").unwrap();
        assert_eq!(ast.blocks.len(), 2);
        assert_eq!(ast.blocks[0].parent, None);
        assert_eq!(ast.blocks[1].parent, Some(0));
        assert_eq!(ast.blocks[0].pos.line, 2);
        assert!(ast.blocks[1].close < ast.blocks[0].close);
        let err = compile("pub fnc main(): int { unsafe { } return 42; }")
            .expect_err("browser guest cannot execute even empty unsafe block");
        assert_eq!(err.code, "E_UNSAFE_DOMAIN");
    }

    #[test]
    fn direct_calls_require_lexical_unsafe_scope() {
        let decl = "pub unsafe fnc trusted(): int { return 42; }\n";
        let valid = format!("{decl}pub fnc main(): int {{ unsafe {{ return trusted(); }} }}");
        let ast = parse(&valid).unwrap();
        assert_eq!(ast.callables.len(), 1);
        assert_eq!(ast.callables[0].name, "trusted");
        assert!(ast.callables[0].requires_unsafe);
        assert_eq!(ast.direct_calls.len(), 1);
        assert!(ast.direct_calls[0].unsafe_block.is_some());
        let invalid = format!("{decl}pub fnc main(): int {{ return trusted(); }}");
        assert_eq!(parse(&invalid).unwrap_err().code, "E_UNSAFE_CONTEXT");
        let ended = format!("{decl}pub fnc main(): int {{ unsafe {{ }} return trusted(); }}");
        assert_eq!(parse(&ended).unwrap_err().code, "E_UNSAFE_CONTEXT");
    }

    #[test]
    fn unsafe_callable_body_is_not_implicitly_unsafe() {
        let src = "pub unsafe fnc inner(): int { return 42; }\n\
                   pub unsafe fnc outer(): int { return inner(); }";
        assert_eq!(parse(src).unwrap_err().code, "E_UNSAFE_CONTEXT");
    }

    #[test]
    fn reject_malformed_and_ambiguous_unsafe_source() {
        for src in [
            "pub fnc main(): int { unsafe; return 42; }",
            "pub fnc main(): int { unsafe (42); }",
            "pub fnc main(): int { unsafe { return 42; }",
            "pub unsafe fnc (): int { return 42; }",
        ] {
            let err = parse(src).unwrap_err();
            assert_eq!(err.code, "E_UNSAFE_SYNTAX", "{src}: {err}");
            assert!(compile(src).is_err(), "no guest artifact on malformed unsafe");
        }
    }

    #[test]
    fn keep_safe_near_misses_and_existing_guest_behavior() {
        for src in ["unsafe_tools", "myunsafe", "_unsafe", "unsafe2",
                    "\"unsafe\"", "/*unsafe*/"] {
            assert!(parse(src).unwrap().first_unsafe.is_none());
        }
        assert_eq!(parse("pub fnc main(): int { val s = \"unsafe { }\"; return 42; }")
                   .unwrap().blocks.len(), 0);
        assert!(compile("pub fnc main(): int { return 42; }").is_ok());
    }

    #[test]
    fn imported_and_prelexed_unsafe_cannot_bypass_guest_gate() {
        let toks = lexer::lex("pub fnc main(): int { unsafe { } return 42; }").unwrap();
        assert_eq!(reject_unlowered(&toks).unwrap_err().code, "E_UNSAFE_DOMAIN");
    }

    #[test]
    fn reject_nested_unsafe_callable_and_unbalanced_delimiters() {
        assert_eq!(parse("pub fnc main(): int { unsafe { unsafe fnc inner(): int { return 42; } } return 42; }")
                   .unwrap_err().code, "E_UNSAFE_CALL");
        assert_eq!(parse("pub fnc main(): int { unsafe { val x = (42]; } return 42; }")
                   .unwrap_err().code, "E_UNSAFE_SYNTAX");
    }
}
