//! First v2 import slice (atto-spec #227). Parses explicit mixed-kind lists
//! into typed selectors; resolution is deliberately separate. No external
//! fetching, module discovery or runtime namespace values.
use crate::diag::{Diagnostic, Pos};
use crate::lexer::{Tok, Token};
use crate::parser::is_reserved;
type R<T> = Result<T, Diagnostic>;
pub(crate) const MAX_ITEMS: usize = 256;
const MAX_PATH: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind { Class, Fnc, Module, Const, Namespace, Actor, Enum, Type, Trait, Interface }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    pub kind: Kind,
    pub path: String,
    pub binding: String,
    pub explicit_alias: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source { Builtin(String), Local(String), Package(String) }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MixedImport { pub selectors: Vec<Selection>, pub source: Source }

/// A dotted alias is a static path, not a member of a runtime value.
pub(crate) fn strict_prefix(prefix: &str, path: &str) -> bool {
    path.strip_prefix(prefix).is_some_and(|suffix| suffix.starts_with('.'))
}

pub(crate) fn obstructs(a: &Selection, b: &Selection) -> bool {
    // A module or namespace may expose a nested static export; defer those
    // overlaps to the authenticated export graph. Other declaration kinds
    // cannot be synthetic namespace prefixes.
    (!matches!(a.kind, Kind::Namespace | Kind::Module)
        && strict_prefix(&a.binding, &b.binding))
        || (!matches!(b.kind, Kind::Namespace | Kind::Module)
        && strict_prefix(&b.binding, &a.binding))
}

struct Cursor<'a> { tokens: &'a [Token], i: usize }
impl Cursor<'_> {
    fn tok(&self) -> &Tok { self.tokens.get(self.i).map_or(&Tok::Eof, |t| &t.tok) }
    fn pos(&self) -> Pos { self.tokens.get(self.i).map_or(Pos::default(), |t| t.pos) }
    fn err(&self, code: &'static str, msg: &str) -> Diagnostic { Diagnostic::new(self.pos(), code, msg) }
    fn take(&mut self, t: &Tok) -> bool {
        if self.tok() == t { self.i += 1; true } else { false }
    }
    fn word(&mut self, s: &str) -> bool { self.take(&Tok::Ident(s.into())) }
    fn need(&mut self, t: &Tok) -> R<()> {
        if self.take(t) { Ok(()) } else { Err(self.err("E_SYNTAX", "malformed mixed import delimiter or selector")) }
    }
    fn ident(&mut self) -> R<String> {
        match self.tok() {
            Tok::Ident(s) if !is_reserved(s) && !s.starts_with("__atto_")
                && s.len() <= 64
                && !matches!(s.as_str(), "pkg" | "enum" | "use" | "declare" | "mtd") => {
                let s = s.clone(); self.i += 1; Ok(s)
            }
            _ => Err(self.err("E_SYNTAX", "expected import identifier")),
        }
    }
    fn kind(&mut self) -> R<Kind> {
        let kind = match self.tok() {
            Tok::Ident(s) => match s.as_str() {
                "class" => Kind::Class, "fnc" => Kind::Fnc, "module" => Kind::Module,
                "const" => Kind::Const, "namespace" => Kind::Namespace, "actor" => Kind::Actor,
                "enum" => Kind::Enum, "type" => Kind::Type, "trait" => Kind::Trait,
                "interface" => Kind::Interface,
                _ => return Err(self.err("E_SYNTAX", "mixed import item needs an explicit kind")),
            },
            _ => return Err(self.err("E_SYNTAX", "mixed import item needs an explicit kind")),
        };
        self.i += 1;
        Ok(kind)
    }
    fn path(&mut self) -> R<Vec<String>> {
        let mut parts = vec![self.ident()?];
        while self.take(&Tok::Dot) {
            if parts.len() >= MAX_PATH { return Err(self.err("E_SOURCE_LIMIT", "import path exceeds eight components")); }
            parts.push(self.ident()?);
        }
        Ok(parts)
    }
    fn selector(&mut self, kind: Kind, out: &mut Vec<Selection>) -> R<()> {
        if out.len() >= MAX_ITEMS { return Err(self.err("E_SOURCE_LIMIT", "too many mixed import selectors")); }
        let parts = self.path()?;
        let explicit_alias = self.word("as");
        let binding = if explicit_alias {
            if kind == Kind::Namespace { self.path()?.join(".") } else { self.ident()? }
        } else if kind == Kind::Namespace { parts.join(".") } else { parts.last().unwrap().clone() };
        if out.iter().any(|s| s.binding == binding) {
            return Err(self.err("E_DUPLICATE", "duplicate mixed import binding"));
        }
        let selection = Selection { kind, path: parts.join("."), binding, explicit_alias };
        if out.iter().any(|s| obstructs(s, &selection)) {
            return Err(self.err("E_DUPLICATE", "non-namespace import obstructs qualified binding"));
        }
        out.push(selection);
        Ok(())
    }
    fn opening_group(&mut self) -> Option<Tok> {
        if self.take(&Tok::LParen) { Some(Tok::RParen) }
        else if self.take(&Tok::LBracket) { Some(Tok::RBracket) }
        else { None }
    }
    fn group(&mut self, kind: Kind, close: Tok, out: &mut Vec<Selection>) -> R<()> {
        self.selector(kind, out)?;
        while self.take(&Tok::Comma) { self.selector(kind, out)?; }
        self.need(&close)
    }
    fn item(&mut self, out: &mut Vec<Selection>) -> R<()> {
        if let Some(close) = self.opening_group() {
            let kind = self.kind()?;
            return self.group(kind, close, out);
        }
        let kind = self.kind()?;
        if let Some(close) = self.opening_group() { self.group(kind, close, out) }
        else { self.selector(kind, out) }
    }
    fn source(&mut self) -> R<Source> {
        let package = self.word("pkg");
        let text = match self.tok() {
            Tok::Str(s) if !s.is_empty() => { let s = s.clone(); self.i += 1; s }
            _ if !package => self.path()?.join("."),
            _ => return Err(self.err("E_SYNTAX", "pkg requires a quoted dependency identity")),
        };
        // Decoded strings can contain escaped controls even if source bytes
        // have no literal NUL. Never forward such selectors to a resolver.
        if text.chars().any(|ch| ch.is_control()) || text.contains('\\') {
            return Err(self.err("E_IMPORT_PATH", "import selector contains control or backslash"));
        }
        if package {
            if text.starts_with("./") || text.starts_with("../") || text.starts_with('/') {
                return Err(self.err("E_SYNTAX", "invalid package selector"));
            }
            if text.chars().any(char::is_whitespace) {
                return Err(self.err("E_IMPORT_PATH", "package selector contains whitespace"));
            }
            if text.contains("://") {
                let authority = text.strip_prefix("https://").and_then(|rest|
                    rest.split(['/', '?', '#']).next());
                if authority.is_none_or(|host| host.is_empty() || host.contains('@')) {
                    return Err(self.err("E_IMPORT_PATH", "remote package requires HTTPS and a credential-free authority"));
                }
            }
            return Ok(Source::Package(text));
        }
        if text.starts_with("./") || text.starts_with("../") {
            return Ok(Source::Local(text));
        }
        if let Some((layer, unit)) = crate::library::normalize(&text) {
            let root = match layer {
                crate::library::Layer::Core => "core",
                crate::library::Layer::Alloc => "alloc",
                crate::library::Layer::Std => "std",
            };
            return Ok(Source::Builtin(format!("{root}.{unit}")));
        }
        Err(self.err("E_IMPORT", "expected registered builtin spelling or relative source"))
    }
}

fn kind_token(t: Option<&Token>) -> bool {
    matches!(t.map(|t| &t.tok), Some(Tok::Ident(s)) if matches!(s.as_str(),
        "class" | "fnc" | "module" | "const" | "namespace" | "actor" |
        "enum" | "type" | "trait" | "interface"))
}

/// Distinguish new mixed syntax from the existing ungrouped homogeneous
/// and direct-path forms. No source lookup is performed during classification.
pub fn is_mixed(tokens: &[Token], start: usize) -> bool {
    if matches!(tokens.get(start).map(|t| &t.tok), Some(Tok::LParen | Tok::LBracket)) {
        return true;
    }
    if !kind_token(tokens.get(start)) { return false; }
    for i in start + 1..tokens.len() {
        match &tokens[i].tok {
            Tok::Ident(s) if s == "from" => break,
            Tok::Semi | Tok::Eof => break,
            Tok::LParen | Tok::LBracket => return true,
            Tok::Comma if kind_token(tokens.get(i + 1)) => return true,
            _ => {}
        }
    }
    false
}

fn finish(c: &mut Cursor<'_>, selectors: Vec<Selection>) -> R<(MixedImport, usize)> {
    if !c.word("from") { return Err(c.err("E_SYNTAX", "mixed import requires from")); }
    let source = c.source()?;
    c.need(&Tok::Semi)?;
    Ok((MixedImport { selectors, source }, c.i))
}

fn parse_list(tokens: &[Token], start: usize, delimited: bool) -> R<(MixedImport, usize)> {
    let mut c = Cursor { tokens, i: start };
    let close = if !delimited { None } else {
        Some(c.opening_group().ok_or_else(|| c.err("E_SYNTAX", "expected import list delimiter"))?)
    };
    let mut selectors = Vec::new();
    loop {
        c.item(&mut selectors)?;
        if let Some(close) = &close {
            if c.take(close) { break; }
            c.need(&Tok::Comma)?;
            if c.take(close) { break; }
        } else if !c.take(&Tok::Comma) { break; }
    }
    finish(&mut c, selectors)
}

/// Parse after the import keyword, returning the offset just after ';'.
/// No mutation or binding is published until the entire list is valid.
pub fn parse_mixed(tokens: &[Token], start: usize) -> R<(MixedImport, usize)> {
    match tokens.get(start).map(|t| &t.tok) {
        Some(Tok::LParen | Tok::LBracket) => {
            // A leading '(' or '[' can be an outer list or the first kind-inside
            // group of an undelimited list. Both alternatives are bounded.
            // Never use successful parsing of a partial clause as admission.
            match parse_list(tokens, start, true) {
                Ok(result) => Ok(result),
                Err(first) => match parse_list(tokens, start, false) {
                    Ok(result) => Ok(result),
                    Err(second) => {
                        // Prefer a specific collision/limit/path failure over a
                        // failed interpretation's generic syntax diagnostic.
                        // For equally specific failures, keep the furthest
                        // original source location (deterministic on ties).
                        if (first.code == "E_SYNTAX" && second.code != "E_SYNTAX")
                            || (first.code == second.code
                                && (second.pos.line, second.pos.col) > (first.pos.line, first.pos.col))
                        { Err(second) } else { Err(first) }
                    },
                },
            }
        }
        _ => parse_list(tokens, start, false),
    }
}

/// Refuse new forms before legacy library expansion or caller-supplied loading.
/// The existing legacy linker remains unchanged; the natural-name builtin namespace lane is the new executable candidate.
/// Mixed lists have a real syntax AST, but not a typed linker yet.
pub fn preflight(source: &str) -> R<()> { preflight_with_local(source, false) }

pub(crate) fn local_function_group(import: &MixedImport) -> bool {
    matches!(&import.source, Source::Local(_)) && import.selectors.iter().all(|s|
        s.kind == Kind::Fnc && !s.path.contains('.') && !s.binding.contains('.'))
}

pub(crate) fn preflight_with_local(source: &str, allow_local: bool) -> R<()> {
    if !source.contains("import") { return Ok(()); }
    if source.len() > crate::SOURCE_CAP {
        return Err(Diagnostic::new(Pos::default(), "E_SOURCE_LIMIT", "source exceeds byte limit"));
    }
    let tokens = crate::lexer::lex(source)?;
    let mut i = 0;
    while i < tokens.len() {
        if !matches!(&tokens[i].tok, Tok::Ident(s) if s == "import") { i += 1; continue; }
        let start = i + 1;
        if is_mixed(&tokens, start) {
            let (import, end) = parse_mixed(&tokens, start)?;
            if allow_local && local_function_group(&import) { i = end; continue; }
            return Err(Diagnostic::new(tokens[i].pos, "E_UNSUPPORTED", "mixed import syntax parsed; typed linking is not implemented"));
        }
        // Leave legacy forms to their existing linkers. The builtin
        // registry separately rejects explicit pkg selectors under reserved roots.
        let mut j = start;
        while j < tokens.len() && !matches!(tokens[j].tok, Tok::Semi | Tok::Eof) { j += 1; }
        i = j.saturating_add(1);
    }
    Ok(())
}
