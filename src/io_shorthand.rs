//! Parse-only I/O direction shorthand preflight for the restricted browser compiler.
//! Retains the existing lexer representation of nested generic '>>' as two Gt
//! tokens. This is NOT typechecking, channel/stream I/O, or select lowering.

use crate::diag::{Diagnostic, Pos};
use crate::lexer::{Tok, Token};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction { Read, Write }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Context { Standalone, SelectArm }

/// Explicit execution mode. Direct operations may statically require suspends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode { Direct, Nonblocking, TryImmediate, Detached }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IoShorthand {
    pub direction: Direction,
    pub context: Context,
    pub mode: Mode,
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
    // Parenthesized, side-effect-free path receivers are valid in v1.
    // Complex computed expressions need the real AST parser/typechecker.
    let (start, end) = if matches!(&tokens[start].tok, Tok::LParen)
        && matches!(&tokens[end - 1].tok, Tok::RParen)
    {
        (start + 1, end - 1)
    } else {
        (start, end)
    };
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

/// A `when` I/O guard belongs to the immediately enclosing select body.
/// Merely seeing the word `when` at a statement boundary must never grant
/// select-only atomicity or alter the parse of ordinary blocks.
fn select_body_for_when(tokens: &[Token], when_start: usize) -> bool {
    let mut braces = Vec::new();
    for (i, token) in tokens.iter().enumerate().take(when_start) {
        match &token.tok {
            Tok::LBrace => braces.push(i),
            Tok::RBrace => { braces.pop(); }
            _ => {}
        }
    }
    let Some(&open) = braces.last() else { return false; };
    if open == 0 { return false; }
    // Direct `select {` and typed `select<Type<...>> {` both own
    // the immediately enclosed guards. Other blocks cannot inherit the
    // permission merely by nesting somewhere inside a select statement.
    if word(&tokens[open - 1]) == Some("select") { return true; }
    if !matches!(&tokens[open - 1].tok, Tok::Gt) { return false; }
    let mut depth = 0usize;
    for j in (0..open).rev() {
        match &tokens[j].tok {
            Tok::Gt => depth += 1,
            Tok::Lt => {
                if depth == 0 { return false; }
                depth -= 1;
                if depth == 0 {
                    return j > 0 && word(&tokens[j - 1]) == Some("select");
                }
            }
            _ => {}
        }
    }
    false
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
        && matches!(&tokens[*cursor].tok, Tok::Dot | Tok::ColonColon)
    {
        if word(&tokens[*cursor + 1]).is_none() { return false; }
        *cursor += 2;
    }
    if *cursor < end && matches!(&tokens[*cursor].tok, Tok::Lt) {
        *cursor += 1;
        if !type_ref(tokens, cursor, end, depth + 1) { return false; }
        while *cursor < end && matches!(&tokens[*cursor].tok, Tok::Comma) {
            *cursor += 1;
            if !type_ref(tokens, cursor, end, depth + 1) { return false; }
        }
        if *cursor >= end || !matches!(&tokens[*cursor].tok, Tok::Gt) { return false; }
        *cursor += 1;
    }
    true
}

fn valid_read_annotation(tokens: &[Token], start: usize, end: usize) -> bool {
    if start >= end || !matches!(&tokens[start].tok, Tok::Colon) { return false; }
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
        .filter(|w| *w != "when" && !crate::parser::is_reserved(w))
        .ok_or_else(|| syntax(pos, "read shorthand requires a binding name"))?;
    if start + 2 != end && !valid_read_annotation(tokens, start + 2, end) {
        return Err(syntax(pos, "read shorthand requires a complete ': Type' annotation (no trailing tokens)"));
    }
    Ok(name.to_owned())
}

/// Bounded syntax preflight. Recognition is distinct from capability typing,
/// suspension-effect verification and guest execution. Unimplemented forms
/// are subsequently rejected by the browser compiler before guest emission.
pub fn inspect(tokens: &[Token]) -> Result<Vec<IoShorthand>, Diagnostic> {
    let mut found = Vec::new();
    for i in 0..tokens.len().saturating_sub(1) {
        let Some(operator) = op_at(tokens, i) else { continue; };
        // Locate the beginning of the containing statement or select arm.
        // This deliberately does not reinterpret embedded numeric operators.
        let mut start = i;
        while start > 0 && !matches!(&tokens[start - 1].tok,
            Tok::Semi | Tok::LBrace | Tok::RBrace | Tok::Arrow)
        {
            start -= 1;
        }
        // This narrowly-scoped syntax family has its own complete parser:
        // nb source >> Future<T> pending |> |f| -> (await f)? >> val U resolved;
        // nb source >> (val U resolved << |f| -> (await f)? <| Future<T>);
        // The typed slot T and resolved binder U must agree on Result.Err
        // propagation; this preflight delegates the full check to the parser.
        // A bare '|> await pending' or legacy '| await pending' is NOT
        // an admitted function stage.
        // Otherwise this legacy direction scanner would diagnose the nested
        // operators as illegal chains before the checked continuation parser
        // can validate them. Skipping this *preflight* does not authorize I/O:
        // the browser compiler still returns E_UNSUPPORTED before guest Wasm.
        // Merely annotating a normal nonblocking binder as Future<T>
        // must NOT bypass the directional scanner. A continuation has both
        // a Future<T> slot AND an explicit await stage in this statement.
        // Otherwise the ordinary 'nb inbox >> val p: Future<T>;' must
        // remain visible as one nonblocking read (and be capability checked).
        let continuation_tokens = tokens.iter().skip(start)
            .take_while(|token| !matches!(token.tok,
                Tok::Semi | Tok::LBrace | Tok::RBrace))
            .collect::<Vec<_>>();
        let is_typed_nb_continuation = tokens.get(start).and_then(word) == Some("nb")
            && !matches!(tokens.get(start + 1).and_then(word), Some("readch" | "writech"))
            && continuation_tokens.iter().any(|token| word(token) == Some("Future"))
            && continuation_tokens.iter().any(|token| word(token) == Some("await"));
        if is_typed_nb_continuation {
            continue;
        }
        let is_when = tokens.get(start).and_then(word) == Some("when");
        let context = if is_when {
            if !select_body_for_when(tokens, start) {
                return Err(syntax(tokens[start].pos,
                    "I/O 'when' guards are only valid directly inside select blocks"));
            }
            Context::SelectArm
        } else {
            Context::Standalone
        };
        let after_when = start + usize::from(is_when);
        let mode = match tokens.get(after_when).and_then(word) {
            Some("nb") => Mode::Nonblocking,
            Some("try") => Mode::TryImmediate,
            Some("do") => Mode::Detached,
            _ => Mode::Direct,
        };
        if context == Context::SelectArm && mode != Mode::Direct {
            return Err(syntax(tokens[after_when].pos,
                "nb/try cannot prefix a select arm; use do nb select or ordinary when I/O"));
        }
        let lhs_start = after_when + usize::from(mode != Mode::Direct);
        let op_pos = tokens[i].pos;
        let rhs_start = i + 2;
        // Explicit readch and writech are complete operations; combining them
        // with another direction guard is malformed, not an implicit cast.
        if matches!(tokens.get(lhs_start).and_then(word), Some("readch" | "writech")) {
            return Err(syntax(op_pos,
                "do not append << or >> to readch/writech; bind the returned value or Future with val/let"));
        }
        // A declaration with an initializer is NOT a reverse read binding.
        // In particular, const x: int = 1 << 3 remains an ordinary expression
        // and must not be mistaken for an effectful const I/O read.
        let has_assignment = tokens[lhs_start..i].iter()
            .any(|token| matches!(token.tok, Tok::Assign));
        let reversed_read = operator == Direction::Write && !has_assignment
            && matches!(tokens.get(lhs_start).and_then(word), Some("const" | "val" | "let"));
        if reversed_read && mode == Mode::Detached {
            return Err(Diagnostic::new(op_pos, "E_IO_DISCARD",
                "detached read cannot bind to caller-local storage; use do readch channel, |outcome| -> void { ... }"));
        }
        if reversed_read && mode != Mode::Direct {
            return Err(syntax(op_pos,
                "nonblocking/immediate reversed read is not in v1; use nb/try source >> val binding"));
        }
        let lhs_path = if reversed_read { None } else { path(tokens, lhs_start, i) };
        if !reversed_read && lhs_path.is_none() { continue; }
        let end = end_at(tokens, rhs_start, context)
            .ok_or_else(|| syntax(op_pos, "I/O shorthand requires ';' or '->' after its operand"))?;
        if end == rhs_start {
            return Err(syntax(op_pos, "I/O shorthand requires a value or binding"));
        }
        if mode == Mode::Detached && (operator == Direction::Read || reversed_read) {
            return Err(Diagnostic::new(op_pos, "E_IO_DISCARD",
                "detached read cannot bind to the caller; use do readch channel, |outcome| -> void { ... }"));
        }
        let (direction, subject, binder) = if reversed_read {
            let binding = read_binder(tokens, lhs_start, i, op_pos)?;
            let source = path(tokens, rhs_start, end)
                .ok_or_else(|| syntax(op_pos,
                    "reversed read requires a qualified readable source; computed expressions are not yet supported"))?;
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
            direction, context, mode, source_or_sink: subject, binder,
            read_binder_on_left: reversed_read, pos: op_pos,
        });
    }
    Ok(found)
}

/// Syntax-only candidate for a continuous producer → consumer connection.
/// This does NOT grant a Readable<T>, Writable<T> or actor capability.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PipeCandidate {
    pub source: String,
    pub sink: String,
    pub pos: Pos,
}

fn pipe_endpoint(tokens: &[Token], start: usize, end: usize) -> Option<String> {
    if let Some(v) = path(tokens, start, end) { return Some(v); }
    // Only a capture-free actor constructor spelling is recognized. Actor
    // capability typing and supervised runtime graph execution are separate.
    if end == start + 4
        && word(tokens.get(start)?) == Some("spawn")
        && matches!(&tokens.get(start + 1)?.tok, Tok::Ident(_))
        && matches!(&tokens.get(start + 2)?.tok, Tok::LParen)
        && matches!(&tokens.get(start + 3)?.tok, Tok::RParen)
    {
        return Some(format!("spawn {}()", word(tokens.get(start + 1)?)?));
    }
    None
}

/// 'pipe' is the ONLY continuous endpoint/actor connection spelling.
/// Directional '<<' and '>>' are single read/write/binding operations.
/// v1 parses exactly one pair then fails closed at the compiler entrypoint:
/// a reference-model pump is not an executable guest transport.
pub fn inspect_pipe(tokens: &[Token]) -> Result<Vec<PipeCandidate>, Diagnostic> {
    let mut out = Vec::new();
    for (i, t) in tokens.iter().enumerate() {
        if word(t) != Some("pipe") { continue; }
        let mut start = i;
        while start > 0 && !matches!(&tokens[start-1].tok,
            Tok::Semi | Tok::LBrace | Tok::RBrace | Tok::Arrow) {
            start -= 1;
        }
        let pos = t.pos;
        if matches!(tokens.get(start).and_then(word),
            Some("nb" | "try" | "do" | "when" | "return")) {
            return Err(syntax(pos,
                "continuous pipe cannot be detached, selected as an atomic guard, or returned implicitly"));
        }
        let end = end_at(tokens, i + 1, Context::Standalone).ok_or_else(||
            syntax(pos, "continuous pipe requires a complete ';'-terminated statement"))?;
        if tokens[i + 1..end].iter().any(|x| word(x) == Some("pipe")) {
            return Err(syntax(pos,
                "chained pipe graphs require a separately typed, supervised runtime"));
        }
        let source = pipe_endpoint(tokens, start, i).ok_or_else(||
            syntax(pos, "left pipe endpoint must be a qualified channel/stream path or spawn Actor()"))?;
        let sink = pipe_endpoint(tokens, i + 1, end).ok_or_else(||
            syntax(pos, "right pipe endpoint must be a qualified channel/stream path or spawn Actor()"))?;
        out.push(PipeCandidate { source, sink, pos });
    }
    Ok(out)
}
