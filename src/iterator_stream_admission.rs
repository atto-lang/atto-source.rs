//! Typed admission boundary for the proposed Iterator/AsyncIterator/Stream APIs.
//!
//! This is NOT a runtime. It never runs callbacks, converts an iterator into
//! Wasm, creates a stream port, or grants guest ownership/loan authority.
//! A later lowering must consume checked compiler IR, not user-supplied facts.
use crate::diag::{Diagnostic, Pos};
use crate::lexer::{Tok, Token};

pub const MAX_STREAM_STAGES: usize = 64;
pub const MAX_STREAM_GENERIC_NESTING: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family { Iterable, Iterator, AsyncIterator, Stream }

impl Family {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "Iterable" => Some(Self::Iterable),
            "Iterator" => Some(Self::Iterator),
            "AsyncIterator" => Some(Self::AsyncIterator),
            "Stream" => Some(Self::Stream),
            _ => None,
        }
    }
    fn is_async(self) -> bool {
        matches!(self, Self::AsyncIterator | Self::Stream)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeCandidate {
    pub family: Family,
    /// Half-open token range of the element type, not a verified nominal type.
    pub element_span: (usize, usize),
    pub pos: Pos,
}

fn error(pos: Pos, code: &'static str, msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new(pos, code, msg)
}

/// Lexical recognition of built-in type requests. Comments and quoted text
/// are already isolated by lex(); no source substring heuristics are used.
/// A type argument is syntactically balanced here, not type-checked or linked.
pub fn inspect(tokens: &[Token]) -> Result<Vec<TypeCandidate>, Diagnostic> {
    let mut candidates = Vec::new();
    for i in 0..tokens.len().saturating_sub(1) {
        let Tok::Ident(name) = &tokens[i].tok else { continue };
        let Some(family) = Family::from_name(name) else { continue };
        if !matches!(tokens[i + 1].tok, Tok::Lt) { continue }
        let mut depth = 1usize;
        let begin = i + 2;
        let mut end = None;
        let mut commas = 0usize;
        for j in begin..tokens.len() {
            match &tokens[j].tok {
                Tok::Lt => {
                    depth += 1;
                    if depth > MAX_STREAM_GENERIC_NESTING {
                        return Err(error(tokens[j].pos, "E_STREAM_TYPE",
                            "iterator generic nesting exceeds 128"));
                    }
                }
                Tok::Gt => {
                    depth -= 1;
                    if depth == 0 { end = Some(j); break; }
                }
                Tok::Comma if depth == 1 => commas += 1,
                Tok::Eof | Tok::Semi | Tok::LBrace | Tok::RBrace if depth == 1 => break,
                _ => {}
            }
        }
        let Some(end) = end else {
            return Err(error(tokens[i].pos, "E_STREAM_TYPE",
                "unterminated iterator/stream generic"));
        };
        if end == begin || commas != 0 {
            return Err(error(tokens[i].pos, "E_STREAM_TYPE",
                "iterator/stream type requires exactly one element type"));
        }
        if end == begin + 1 && matches!(&tokens[begin].tok,
                Tok::Ident(value) if value == "void" || value == "never") {
            return Err(error(tokens[begin].pos, "E_STREAM_TYPE",
                "iterator elements cannot have void or never type"));
        }
        candidates.push(TypeCandidate {
            family, element_span: (begin, end), pos: tokens[i].pos,
        });
    }
    Ok(candidates)
}

/// These mode/effect facts are trusted ONLY after a full typed checker has
/// validated the corresponding callable and its transitive captures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallbackMode { ReadonlySync, ConsumingSync, Async }

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stage {
    Map { input: String, output: String, mode: CallbackMode,
        consuming_proven: bool },
    Filter { input: String, result: String, mode: CallbackMode },
    Take { count: u64 },
    Skip { count: u64 },
    Collect { max_items: Option<u64> },
    Fold { input: String, accumulator: String, result: String,
        mode: CallbackMode },
}

/// Typed stage descriptors. No executable callbacks, buffer allocations or
/// guest-visible reflective metadata are retained in this plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PipelinePlan {
    pub family: Family,
    pub item_type: String,
    pub stages: Vec<Stage>,
    pub terminal_result: Option<String>,
    /// A take(n) proves an upper bound; skip(n) never proves finiteness.
    pub bounded_items: Option<u64>,
}

impl PipelinePlan {
    pub fn new(family: Family, element_type: &str) -> Self {
        Self {
            family, item_type: element_type.to_owned(), stages: Vec::new(),
            terminal_result: None, bounded_items: None,
        }
    }

    pub fn apply(&mut self, stage: Stage, pos: Pos) -> Result<(), Diagnostic> {
        if self.family == Family::Iterable {
            return Err(error(pos, "E_STREAM_TYPE",
                "Iterable<T> must be converted to an Iterator<T> using iter()"));
        }
        if self.stages.len() >= MAX_STREAM_STAGES {
            return Err(error(pos, "E_STREAM_QUOTA",
                "pipeline exceeds 64 statically admitted stages"));
        }
        if self.terminal_result.is_some() {
            return Err(error(pos, "E_STREAM_TERMINAL",
                "fold/collect already terminated this pipeline"));
        }
        match &stage {
            Stage::Map { input, output, mode, consuming_proven } => {
                if input != &self.item_type || output.is_empty() || output == "void" {
                    return Err(error(pos, "E_STREAM_TYPE",
                        "map input/output element type mismatch"));
                }
                if *mode == CallbackMode::Async {
                    return Err(error(pos, "E_STREAM_CALLBACK_EFFECT",
                        "map does not await callbacks; map_async must be explicit"));
                }
                if *mode == CallbackMode::ConsumingSync && !consuming_proven {
                    return Err(error(pos, "E_ITERATOR_OWNERSHIP",
                        "consuming map requires a checked ownership transfer"));
                }
                self.item_type.clone_from(output);
            }
            Stage::Filter { input, result, mode } => {
                if input != &self.item_type || result != "bool" {
                    return Err(error(pos, "E_STREAM_TYPE",
                        "filter must accept T and return bool"));
                }
                if *mode != CallbackMode::ReadonlySync {
                    return Err(error(pos, "E_STREAM_CALLBACK_EFFECT",
                        "filter requires a readonly synchronous predicate"));
                }
            }
            Stage::Take { count } => {
                self.bounded_items = Some(self.bounded_items.map_or(*count,
                    |old| old.min(*count)));
            }
            Stage::Skip { .. } => {}
            Stage::Collect { max_items } => {
                let cap = (*max_items).or(self.bounded_items)
                    .ok_or_else(|| error(pos, "E_STREAM_UNBOUNDED_COLLECT",
                        "collect requires max_items or a proved take(n) bound"))?;
                self.bounded_items = Some(cap);
                let value = format!("Result<List<{}>,CollectError>", self.item_type);
                self.terminal_result = Some(if self.family.is_async() {
                    format!("Future<{value}>")
                } else { value });
            }
            Stage::Fold { input, accumulator, result, mode } => {
                if input != &self.item_type || accumulator != result || result.is_empty() {
                    return Err(error(pos, "E_STREAM_TYPE",
                        "fold callback must preserve its accumulator type"));
                }
                if *mode != CallbackMode::ReadonlySync {
                    return Err(error(pos, "E_STREAM_CALLBACK_EFFECT",
                        "fold callback cannot be async or consume a borrowed item"));
                }
                self.terminal_result = Some(if self.family.is_async() {
                    format!("Future<{result}>")
                } else { result.clone() });
            }
        }
        self.stages.push(stage);
        Ok(())
    }
}
