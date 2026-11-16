//! Parse-only I/O direction shorthand preflight for the restricted browser compiler.
//! Retains the existing lexer representation of nested generic '>>' as two Gt
//! tokens. This is NOT typechecking, channel/stream I/O, or select lowering.

use crate::diag::{Diagnostic, Pos};
use crate::lexer::{Tok, Token};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction { Read, Write }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Context { Standalone, SelectArm }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IoShorthand {
    pub direction: Direction,
    pub context: Context,
    pub source_or_sink: String,
    pub binder: Option<String>,
    /// True for reversed read-binding syntax with the binder on the left.
    pub read_binder_on_left: bool,
    /// Operator start position. Payload is intentionally not evaluated.
    pub pos: Pos,
}

fn word(tok: &Token) -> Option<&str> {
    match &tok.tok { Tok::Ident(s) => Some(s), _ => None }
}

fn op_at(tokens: &[Token], i: usize) -> Option<Direction> {
    let (a, b) = (tokens.get(i)?, tokens.get(i + 1)?);
    if a.pos.line != b.pos.line || a.pos.col.checked_add(1) != Some(b.pos.col) {
        return None;
    }
    match (&a.tok, &b.tok) {
        (Tok::Lt, Tok::Lt) => Some(Direction::Write),
        (Tok::Gt, Tok::Gt) => Some(Direction::Read),
        _ => None,
    }
}

fn path(tokens: &[Token], start: usize, end: usize) -> Option<String> {
    if start >= end { return None; }
    let mut components = Vec::new();
    let mut i = start;
    while i < end {
        components.push(word(&tokens[i])?.to_owned());
        i += 1;
        if i == end { break; }
        if !matches!(&tokens[i].tok, Tok::Dot) { return None; }
        i += 1;
        if i == end { return None; }
    }
    Some(components.join("."))
}

fn syntax(pos: Pos, message: &str) -> Diagnostic {
    Diagnostic::new(pos, "E_IO_SYNTAX", message)
}

fn end_at(tokens: &[Token], start: usize, context: Context) -> Option<usize> {
    let mut parens = 0u16;
    let mut brackets = 0u16;
    for i in start..tokens.len() {
        match &tokens[i].tok {
            Tok::LParen => parens = parens.checked_add(1)?,
            Tok::RParen => parens = parens.checked_sub(1)?,
            Tok::LBracket => brackets = brackets.checked_add(1)?,
            Tok::RBracket => brackets = brackets.checked_sub(1)?,
            Tok::Semi if parens == 0 && brackets == 0 && context == Context::Standalone => return Some(i),
            Tok::Arrow if parens == 0 && brackets == 0 && context == Context::SelectArm => return Some(i),
            Tok::Eof | Tok::RBrace if parens == 0 && brackets == 0 => return None,
            _ => {}
        }
    }
    None
}

/// Parse the entire optional type annotation, never just its first token.
/// The preflight intentionally supports only path types with nested generic
/// arguments. Unsupported richer type grammars must be implemented in the
/// real parser instead of silently accepting unchecked trailing tokens.
fn type_ref(tokens: &[Token], cursor: &mut usize, end: usize, depth: usize) -> bool {
    if depth > 64 || *cursor >= end || word(&tokens[*cursor]).is_none() {
        return false;
    }
    *cursor += 1;
    while *cursor + 1 < end
        && matches!(tokens[*cursor].tok, Tok::Dot | Tok::ColonColon)
    {
        if word(&tokens[*cursor + 1]).is_none() { return false; }
        *cursor += 2;
    }
    if *cursor < end && matches!(tokens[*cursor].tok, Tok::Lt) {
        *cursor += 1;
        if !type_ref(tokens, cursor, end, depth + 1) { return false; }
        while *cursor < end && matches!(tokens[*cursor].tok, Tok::Comma) {
            *cursor += 1;
            if !type_ref(tokens, cursor, end, depth + 1) { return false; }
        }
        if *cursor >= end || !matches!(tokens[*cursor].tok, Tok::Gt) { return false; }
        *cursor += 1;
    }
    true
}

fn valid_read_annotation(tokens: &[Token], start: usize, end: usize) -> bool {
    if start >= end || !matches!(tokens[start].tok, Tok::Colon) { return false; }
    let mut cursor = start + 1;
    type_ref(tokens, &mut cursor, end, 0) && cursor == end
}

/// Read binding syntax is a declaration, never an assignment.
/// Compile-time const cannot be initialized by effectful I/O.
fn read_binder(tokens: &[Token], start: usize, end: usize, pos: Pos) -> Result<String, Diagnostic> {
    let qualifier = tokens.get(start).and_then(word);
    if qualifier == Some("const") {
        return Err(Diagnostic::new(pos, "E_CONST_EFFECT",
            "const requires a compile-time initializer; use val or let for runtime I/O reads"));
    }
    if !matches!(qualifier, Some("val" | "let")) {
        return Err(syntax(pos, "read shorthand requires val or let followed by a binding name"));
    }
    let name = tokens.get(start + 1).and_then(word)
        .filter(|w| !matches!(*w,
            "const" | "val" | "let" | "when" | "return" | "select" | "do" | "writech" | "readch"))
        .ok_or_else(|| syntax(pos, "read shorthand requires a binding name"))?;
    if start + 2 != end && !valid_read_annotation(tokens, start + 2, end) {
        return Err(syntax(pos, "read shorthand requires a complete ': Type' annotation (no trailing tokens)"));
    }
    Ok(name.to_owned())
}

/// Bounded syntax preflight. Recognition is distinct from capability typing
/// and guest execution; downstream browser compilation fails closed.
pub fn inspect(tokens: &[Token]) -> Result<Vec<IoShorthand>, Diagnostic> {
    let mut found = Vec::new();
    for i in 0..tokens.len().saturating_sub(1) {
        let Some(operator) = op_at(tokens, i) else { continue; };
        // Only statement/when-arm leading expressions are admitted here.
        // A real expression parser must handle parenthesized and computed IO.
        let mut start = i;
        while start > 0 && !matches!(&tokens[start - 1].tok,
            Tok::Semi | Tok::LBrace | Tok::RBrace | Tok::Arrow)
        {
            start -= 1;
        }
        let context = if tokens.get(start).and_then(word) == Some("when") {
            Context::SelectArm
        } else {
            Context::Standalone
        };
        let lhs_start = start + usize::from(context == Context::SelectArm);
        let op_pos = tokens[i].pos;
        let rhs_start = i + 2;
        // A leading declaration binder to the left of << means INPUT;
        // an expression on the left of << means OUTPUT. Never guess from
        // runtime values, type names, or operator overloading.
        let reversed_read = operator == Direction::Write
            && matches!(tokens.get(lhs_start).and_then(word), Some("const" | "val" | "let"));
        let lhs_path = if reversed_read { None } else { path(tokens, lhs_start, i) };
        if !reversed_read && lhs_path.is_none() { continue; }
        let end = end_at(tokens, rhs_start, context)
            .ok_or_else(|| syntax(op_pos, "I/O shorthand requires ';' or '->' after its operand"))?;
        if end == rhs_start {
            return Err(syntax(op_pos, "I/O shorthand requires a value or binding"));
        }
        let (direction, subject, binder) = if reversed_read {
            let binding = read_binder(tokens, lhs_start, i, op_pos)?;
            let source = path(tokens, rhs_start, end)
                .ok_or_else(|| syntax(op_pos,
                    "reversed read shorthand requires a qualified readable source; computed expressions are not yet supported"))?;
            (Direction::Read, source, Some(binding))
        } else if operator == Direction::Read {
            let binding = read_binder(tokens, rhs_start, end, op_pos)?;
            (Direction::Read, lhs_path.unwrap(), Some(binding))
        } else {
            for j in rhs_start..end {
                if op_at(tokens, j).is_some() {
                    return Err(syntax(op_pos, "chained I/O direction operators are not allowed in v1"));
                }
            }
            (Direction::Write, lhs_path.unwrap(), None)
        };
        found.push(IoShorthand {
            direction, context, source_or_sink: subject, binder,
            read_binder_on_left: reversed_read, pos: op_pos,
        });
    }
    Ok(found)
}
