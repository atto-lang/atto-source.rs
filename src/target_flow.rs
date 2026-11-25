//! Conservative, structured control-flow facts for out-of-line actor receivers.
//! This parser is intentionally NOT an executable body lowering. Unsupported
//! statements/expressions invalidate completeness instead of inventing proofs.
use crate::diag::{Diagnostic, Pos};
use crate::lexer::{Tok, Token};
use crate::target_syntax::{ReceiverBodyTokens, TargetHeaders};
use crate::target_typed::{TargetType, TypedActorModule, TypedReceiver};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReceiverFlow {
    pub receiver: usize,
    pub branches: usize,
    pub returns: usize,
    pub abrupt_exits: usize,
    /// None: unsupported flow may change reachability. Some(true): a normal
    /// path falls off the end. Some(false): all analyzed paths exit.
    pub normal_fallthrough: Option<bool>,
    /// Never implies emitted code or validated ownership lifetimes.
    pub fully_structured: bool,
}

#[derive(Default)]
struct BlockFacts {
    branches: usize,
    returns: usize,
    abrupt_exits: usize,
    unknown: bool,
    falls_through: bool,
}

struct Reader<'a> {
    tokens: &'a [Token],
    index: usize,
    receiver: &'a TypedReceiver,
}

impl Reader<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.index).map(|t| &t.tok)
    }
    fn word(&self, s: &str) -> bool {
        matches!(self.peek(), Some(Tok::Ident(w)) if w == s)
    }
    fn pos(&self) -> Pos {
        self.tokens.get(self.index).map_or(Pos::default(), |t| t.pos)
    }
    fn err(&self, code: &'static str, s: &'static str) -> Diagnostic {
        Diagnostic::new(self.pos(), code, s)
    }
    fn expect_brace(&mut self) -> Result<(), Diagnostic> {
        if !matches!(self.peek(), Some(Tok::LBrace)) {
            return Err(self.err("E_SYNTAX", "expected a receiver control-flow block"));
        }
        self.index += 1;
        Ok(())
    }
    fn known_bool_condition(&self, tokens: &[Token], depth: usize)
        -> Result<bool, Diagnostic> {
        if depth > 128 || tokens.is_empty() { return Ok(false); }
        match tokens {
            [Token { tok: Tok::Ident(s), .. }] if s == "true" || s == "false" =>
                Ok(true),
            [Token { tok: Tok::Ident(s), pos }] => {
                let Some(param) = self.receiver.params.iter().find(|p| p.name == *s)
                    else { return Ok(false); };
                if param.ty != TargetType::Bool {
                    return Err(Diagnostic::new(*pos, "E_TYPE",
                        "receiver condition must be boolean"));
                }
                Ok(true)
            }
            [Token {tok: Tok::Bang, ..}, rest @ ..] =>
                self.known_bool_condition(rest, depth + 1),
            [Token {tok: Tok::LParen, ..}, inner @ .., Token {tok: Tok::RParen, ..}] =>
                self.known_bool_condition(inner, depth + 1),
            [Token {tok: Tok::Int(_), pos}] | [Token {tok: Tok::Float(_), pos}] |
            [Token {tok: Tok::Str(_), pos}] =>
                Err(Diagnostic::new(*pos, "E_TYPE",
                    "receiver condition must be boolean")),
            _ => Ok(false),
        }
    }
    /// Skip one statement with correctly nested delimiters. This does not
    /// claim anything about effects, aliasing, or exits within that statement.
    fn skip_unknown(&mut self) -> Result<(), Diagnostic> {
        let mut expected = Vec::<Tok>::new();
        let mut had_brace = false;
        while self.index < self.tokens.len() {
            match self.peek() {
                Some(Tok::RBrace) if expected.is_empty() => break,
                Some(Tok::Semi) if expected.is_empty() => {
                    self.index += 1;
                    break;
                }
                Some(Tok::LParen) => expected.push(Tok::RParen),
                Some(Tok::LBracket) => expected.push(Tok::RBracket),
                Some(Tok::LBrace) => {
                    had_brace = true;
                    expected.push(Tok::RBrace);
                }
                Some(Tok::RParen | Tok::RBracket | Tok::RBrace) => {
                    let actual = self.peek().cloned().unwrap();
                    if expected.pop().as_ref() != Some(&actual) {
                        return Err(self.err("E_SYNTAX",
                            "mismatched opaque receiver statement delimiter"));
                    }
                    self.index += 1;
                    if had_brace && expected.is_empty() { break; }
                    continue;
                }
                _ => {}
            }
            if expected.len() > 128 {
                return Err(self.err("E_NESTING", "receiver statement nesting limit"));
            }
            self.index += 1;
        }
        if !expected.is_empty() {
            return Err(self.err("E_SYNTAX", "unclosed receiver statement delimiter"));
        }
        Ok(())
    }
    fn consume_to_semicolon(&mut self) -> Result<(), Diagnostic> {
        let mut expected = Vec::<Tok>::new();
        while self.index < self.tokens.len() {
            match self.peek() {
                Some(Tok::Semi) if expected.is_empty() => {
                    self.index += 1;
                    return Ok(());
                }
                Some(Tok::RBrace) if expected.is_empty() =>
                    return Err(self.err("E_SYNTAX", "expected ';' after receiver exit")),
                Some(Tok::LParen) => expected.push(Tok::RParen),
                Some(Tok::LBracket) => expected.push(Tok::RBracket),
                Some(Tok::LBrace) => expected.push(Tok::RBrace),
                Some(Tok::RParen | Tok::RBracket | Tok::RBrace) => {
                    let actual = self.peek().cloned().unwrap();
                    if expected.pop().as_ref() != Some(&actual) {
                        return Err(self.err("E_SYNTAX",
                            "mismatched receiver exit delimiter"));
                    }
                }
                _ => {}
            }
            if expected.len() > 128 {
                return Err(self.err("E_NESTING", "receiver exit nesting limit"));
            }
            self.index += 1;
        }
        Err(self.err("E_SYNTAX", "unclosed receiver exit statement"))
    }
    fn parse_if(&mut self, depth: usize) -> Result<Option<BlockFacts>, Diagnostic> {
        // Only brace-delimited if expressions are admitted to the current CFG
        // profile; other legal language spellings remain opaque.
        let original = self.index;
        self.index += 1;
        let start = self.index;
        let mut groups = Vec::<Tok>::new();
        while self.index < self.tokens.len() {
            match self.peek() {
                Some(Tok::LBrace) if groups.is_empty() => break,
                Some(Tok::RBrace | Tok::Semi) if groups.is_empty() => {
                    self.index = original;
                    return Ok(None);
                }
                Some(Tok::LParen) => groups.push(Tok::RParen),
                Some(Tok::LBracket) => groups.push(Tok::RBracket),
                Some(Tok::RParen | Tok::RBracket) => {
                    let close = self.peek().cloned().unwrap();
                    if groups.pop().as_ref() != Some(&close) {
                        self.index = original;
                        return Ok(None);
                    }
                }
                _ => {}
            }
            if groups.len() > 128 {
                return Err(self.err("E_NESTING", "receiver condition nesting limit"));
            }
            self.index += 1;
        }
        if !matches!(self.peek(), Some(Tok::LBrace)) {
            self.index = original;
            return Ok(None);
        }
        let condition = &self.tokens[start..self.index];
        let condition_proved = self.known_bool_condition(condition, 0)?;
        let then_branch = self.parse_block(depth + 1)?;
        let else_branch = if self.word("else") {
            self.index += 1;
            if self.word("if") {
                match self.parse_if(depth + 1)? {
                    Some(b) => Some(b),
                    None => {
                        let mut unknown = BlockFacts::default();
                        unknown.unknown = true;
                        unknown.falls_through = true;
                        self.skip_unknown()?;
                        Some(unknown)
                    }
                }
            } else if matches!(self.peek(), Some(Tok::LBrace)) {
                Some(self.parse_block(depth + 1)?)
            } else {
                self.skip_unknown()?;
                let mut unknown = BlockFacts::default();
                unknown.unknown = true;
                unknown.falls_through = true;
                Some(unknown)
            }
        } else { None };
        let else_falls = else_branch.as_ref().map_or(true, |b| b.falls_through);
        Ok(Some(BlockFacts {
            branches: 1 + then_branch.branches
                + else_branch.as_ref().map_or(0, |b| b.branches),
            returns: then_branch.returns + else_branch.as_ref().map_or(0, |b| b.returns),
            abrupt_exits: then_branch.abrupt_exits
                + else_branch.as_ref().map_or(0, |b| b.abrupt_exits),
            unknown: !condition_proved || then_branch.unknown
                || else_branch.as_ref().is_some_and(|b| b.unknown),
            falls_through: then_branch.falls_through || else_falls,
        }))
    }
    fn parse_block(&mut self, depth: usize) -> Result<BlockFacts, Diagnostic> {
        if depth > 128 {
            return Err(self.err("E_NESTING", "receiver CFG nesting limit"));
        }
        self.expect_brace()?;
        let mut facts = BlockFacts { falls_through: true, ..BlockFacts::default() };
        while self.index < self.tokens.len()
            && !matches!(self.peek(), Some(Tok::RBrace)) {
            let before = self.index;
            let stmt = if self.word("if") {
                if let Some(branch) = self.parse_if(depth)? {
                    branch
                } else {
                    self.skip_unknown()?;
                    BlockFacts { unknown: true, falls_through: true,
                        ..BlockFacts::default() }
                }
            } else if self.word("return") {
                self.index += 1;
                self.consume_to_semicolon()?;
                BlockFacts { returns: 1, falls_through: false,
                    ..BlockFacts::default() }
            } else if self.word("throw") || self.word("raise") || self.word("panic") {
                self.index += 1;
                self.consume_to_semicolon()?;
                BlockFacts { abrupt_exits: 1, falls_through: false,
                    ..BlockFacts::default() }
            } else {
                self.skip_unknown()?;
                BlockFacts { unknown: true, falls_through: true,
                    ..BlockFacts::default() }
            };
            if self.index == before {
                return Err(self.err("E_SYNTAX", "receiver statement did not advance"));
            }
            facts.branches += stmt.branches;
            facts.returns += stmt.returns;
            facts.abrupt_exits += stmt.abrupt_exits;
            facts.unknown |= stmt.unknown;
            if facts.falls_through {
                facts.falls_through = stmt.falls_through;
            }
        }
        if !matches!(self.peek(), Some(Tok::RBrace)) {
            return Err(self.err("E_SYNTAX", "unclosed receiver CFG block"));
        }
        self.index += 1;
        Ok(facts)
    }
}

/// Produce only the CFG facts justified by the supported syntax subset.
/// A non-void method with a proved if-return branch and an uncovered normal
/// path is rejected. Old forward-only stubs remain unproved, not executable.
pub fn analyze_receiver_body(headers: &TargetHeaders, module: &TypedActorModule,
    body: &ReceiverBodyTokens) -> Result<ReceiverFlow, Diagnostic> {
    let receiver = module.receivers.get(body.receiver)
        .ok_or_else(|| Diagnostic::new(Pos::default(), "E_TYPE",
            "invalid receiver CFG index"))?;
    let mut reader = Reader { tokens: &body.tokens, index: 0, receiver };
    let facts = reader.parse_block(0)?;
    if reader.index != reader.tokens.len() {
        return Err(reader.err("E_SYNTAX", "trailing receiver CFG tokens"));
    }
    if !facts.unknown && facts.branches > 0 && facts.returns > 0
        && receiver.returns != TargetType::Void && facts.falls_through {
        let pos = headers.receivers.get(body.receiver)
            .map_or(Pos::default(), |h| h.pos);
        return Err(Diagnostic::new(pos, "E_MISSING_RETURN",
            "not all receiver control-flow paths return a value"));
    }
    Ok(ReceiverFlow {
        receiver: body.receiver,
        branches: facts.branches,
        returns: facts.returns,
        abrupt_exits: facts.abrupt_exits,
        normal_fallthrough: if facts.unknown { None } else { Some(facts.falls_through) },
        fully_structured: !facts.unknown,
    })
}

fn infer_argument(tokens: &[Token], caller: &TypedReceiver, module: &TypedActorModule,
    depth: usize) -> Result<Option<TargetType>, Diagnostic> {
    if depth > 128 { return Ok(None); }
    match tokens {
        [] => Ok(None),
        [Token {tok: Tok::Int(None), pos}] =>
            Err(Diagnostic::new(*pos, "E_INTEGER_RANGE",
                "positive 2^63 cannot be passed as a signed int")),
        [Token {tok: Tok::Int(Some(_)), ..}] |
        [Token {tok: Tok::Minus, ..}, Token {tok: Tok::Int(_), ..}] =>
            Ok(Some(TargetType::Int)),
        [Token {tok: Tok::Float(_), ..}] => Ok(Some(TargetType::Float)),
        [Token {tok: Tok::Str(_), ..}] => Ok(Some(TargetType::Str)),
        [Token {tok: Tok::Ident(s), ..}] if s == "true" || s == "false" =>
            Ok(Some(TargetType::Bool)),
        [Token {tok: Tok::Ident(s), ..}] =>
            Ok(caller.params.iter().find(|p| p.name == *s).map(|p| p.ty.clone())),
        [Token {tok: Tok::LParen, ..}, inner @ ..,
         Token {tok: Tok::RParen, ..}] =>
            infer_argument(inner, caller, module, depth + 1),
        [Token {tok: Tok::Ident(owner), ..}, Token {tok: Tok::Dot, ..},
         Token {tok: Tok::Ident(method), ..}, Token {tok: Tok::LParen, ..},
         Token {tok: Tok::RParen, ..}] if owner == "self" => {
            Ok(module.receivers.iter().find(|m| m.actor == caller.actor
                && m.method == *method && m.arity == 0).map(|m| m.returns.clone()))
        }
        _ => Ok(None),
    }
}

/// Validate every statically recognizable argument in a nominal call.
/// Complex expressions remain unproved; never use this as an execution gate.
pub fn check_receiver_arguments(headers: &TargetHeaders, module: &TypedActorModule)
    -> Result<Vec<bool>, Diagnostic> {
    let mut unknown_per_receiver = vec![false; module.receivers.len()];
    for call in &headers.receiver_calls {
        let caller = module.receivers.get(call.caller).ok_or_else(|| {
            Diagnostic::new(call.pos, "E_TYPE", "invalid caller receiver index")
        })?;
        let callee = module.receivers.get(call.callee).ok_or_else(|| {
            Diagnostic::new(call.pos, "E_TYPE", "invalid callee receiver index")
        })?;
        if caller.actor != callee.actor {
            return Err(Diagnostic::new(call.pos, "E_ISOLATION",
                "nominal receiver calls cannot cross actor ownership domains"));
        }
        if call.arguments.len() != callee.params.len() {
            return Err(Diagnostic::new(call.pos, "E_ARITY",
                "receiver call argument arity does not match signature"));
        }
        for (arg, param) in call.arguments.iter().zip(&callee.params) {
            match infer_argument(arg, caller, module, 0)? {
                Some(actual) if actual != param.ty =>
                    return Err(Diagnostic::new(arg.first().map_or(call.pos, |t| t.pos),
                        "E_TYPE", format!(
                            "receiver argument '{}' expects {:?}, got {:?}",
                            param.name, param.ty, actual))),
                Some(_) => {}
                None => unknown_per_receiver[call.caller] = true,
            }
        }
    }
    Ok(unknown_per_receiver)
}
