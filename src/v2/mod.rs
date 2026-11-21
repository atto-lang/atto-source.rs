//! General ("v2") pipeline: async tasks, channels, pattern matching, closures,
//! in-guest actors and pipelines. It emits the same guest contract as the
//! legacy pipeline (`atto.browser.guest.v2`) and needs no extra host imports:
//! the scheduler and runtime are guest code (`prelude.atto`).

pub mod ast;
pub mod check;
pub mod emit;
pub mod ir;
pub mod parse;
pub mod types;

use crate::diag::Diagnostic;
use crate::lexer::Token;

const PRELUDE: &str = include_str!("prelude.atto");

/// The shared lexer emits `&&` and `||` as two adjacent single-character
/// tokens (the legacy parser depends on that). The v2 grammar was written for
/// fused operator tokens, so fuse exactly-adjacent pairs here, inside v2 only.
/// `^^` never reaches this point: the shared lexer rejects it.
pub fn fuse_operators(toks: Vec<Token>) -> Vec<Token> {
    use crate::lexer::Tok;
    let mut out: Vec<Token> = Vec::with_capacity(toks.len());
    for t in toks {
        if let Some(prev) = out.last_mut() {
            let adjacent = prev.pos.line == t.pos.line && prev.pos.col + 1 == t.pos.col;
            let fused = match (&prev.tok, &t.tok) {
                (Tok::Amp, Tok::Amp) if adjacent => Some(Tok::AmpAmp),
                (Tok::Pipe, Tok::Pipe) if adjacent => Some(Tok::PipePipe),
                _ => None,
            };
            if let Some(tok) = fused {
                prev.tok = tok;
                continue;
            }
        }
        out.push(t);
    }
    out
}

pub fn compile(toks: Vec<Token>) -> Result<Vec<u8>, Diagnostic> {
    let program = parse::Parser::new(fuse_operators(toks)).parse_program()?;
    let prelude_toks = fuse_operators(crate::lexer::lex(PRELUDE).expect("the runtime prelude lexes"));
    let prelude = parse::Parser::new(prelude_toks).parse_program().expect("the runtime prelude parses");
    let module = check::check(&program, &prelude)?;
    Ok(emit::emit(&module))
}
