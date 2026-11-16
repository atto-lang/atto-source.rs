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

pub fn compile(toks: Vec<Token>) -> Result<Vec<u8>, Diagnostic> {
    let program = parse::Parser::new(toks).parse_program()?;
    let prelude_toks = crate::lexer::lex(PRELUDE).expect("the runtime prelude lexes");
    let prelude = parse::Parser::new(prelude_toks).parse_program().expect("the runtime prelude parses");
    let module = check::check(&program, &prelude)?;
    Ok(emit::emit(&module))
}
