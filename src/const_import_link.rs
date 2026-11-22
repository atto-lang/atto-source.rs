//! First *executable* bounded relative-file constant import profile.
//!
//! Only direct, scalar, literal, one-hop `define [pub] const` declarations in
//! caller-supplied relative .atto files are admitted. The importer substitutes
//! literal tokens before the ordinary production type checker and Wasm emitter.
//! No compile-time host environment read, partial package resolver, dynamic
//! initialization, mutable storage or unsafe namespace alias is invented.
//! Every unsupported form fails before publishing a guest artifact.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use crate::diag::{Diagnostic, Pos};
use crate::lexer::{self, Tok, Token};

type R<T> = Result<T, Diagnostic>;
const MAX_FILES: usize = 16;
const MAX_BINDINGS: usize = 256;
const MAX_CONST_TOKENS: usize = 256;

#[derive(Clone)]
struct ConstExport {
    public: bool,
    value: Vec<Tok>,
}

fn fail(pos: Pos, code: &'static str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(pos, code, message)
}

fn word(tokens: &[Token], i: usize, name: &str) -> bool {
    matches!(tokens.get(i).map(|t| &t.tok), Some(Tok::Ident(s)) if s == name)
}

fn get_ident(tokens: &[Token], i: &mut usize, pos: Pos) -> R<String> {
    match tokens.get(*i) {
        Some(Token { tok: Tok::Ident(s), .. }) if !crate::parser::is_reserved(s)
            && !matches!(s.as_str(), "from" | "as" | "const" | "import" | "pkg" | "define" | "pub") => {
            *i += 1;
            Ok(s.clone())
        }
        _ => Err(fail(pos, "E_SYNTAX", "expected a non-keyword identifier")),
    }
}

fn expect_word(tokens: &[Token], i: &mut usize, word_name: &str, pos: Pos) -> R<()> {
    if !word(tokens, *i, word_name) {
        return Err(fail(pos, "E_SYNTAX", format!("expected '{word_name}'")));
    }
    *i += 1;
    Ok(())
}

fn source_path(logical: &Path, selector: &str, pos: Pos) -> R<PathBuf> {
    if logical.is_absolute() || !(selector.starts_with("./") || selector.starts_with("../"))
        || selector.contains('\\') || selector.contains('\0') {
        return Err(fail(pos, "E_IMPORT_PATH",
            "constant file import needs a project-relative ./ or ../ .atto path"));
    }
    let mut normalized = PathBuf::new();
    for c in logical.parent().unwrap_or_else(|| Path::new("")).components()
        .chain(Path::new(selector).components()) {
        match c {
            Component::CurDir => {},
            Component::Normal(name) => normalized.push(name),
            Component::ParentDir if normalized.pop() => {},
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(fail(pos, "E_IMPORT_PATH", "constant import escapes project root"));
            }
        }
    }
    if normalized.extension().and_then(|x| x.to_str()) != Some("atto") {
        return Err(fail(pos, "E_IMPORT_PATH", "constant import must target .atto"));
    }
    Ok(normalized)
}

fn reject_reserved(tokens: &[Token]) -> R<()> {
    for item in tokens {
        if let Tok::Ident(name) = &item.tok {
            if name.starts_with("__atto_file_import_")
                || name.starts_with("__atto_internal_module_") {
                return Err(fail(item.pos, "E_RESERVED",
                    "reserved compiler import/module identifier cannot occur in source"));
            }
        }
    }
    Ok(())
}

fn file_constants(src: &str) -> R<BTreeMap<String, ConstExport>> {
    if src.len() > crate::SOURCE_CAP {
        return Err(fail(Pos::default(), "E_SOURCE_LIMIT", "imported file exceeds byte limit"));
    }
    let tokens = lexer::lex(src)?;
    reject_reserved(&tokens)?;
    let mut i = 0usize;
    let mut exports = BTreeMap::new();
    while tokens.get(i).is_some_and(|t| t.tok != Tok::Eof) {
        let pos = tokens[i].pos;
        expect_word(&tokens, &mut i, "define", pos)?;
        let public = if word(&tokens, i, "pub") { i += 1; true } else { false };
        expect_word(&tokens, &mut i, "const", pos)?;
        let ty = get_ident(&tokens, &mut i, pos)?;
        if !matches!(ty.as_str(), "int" | "bool" | "string") {
            return Err(fail(pos, "E_UNSUPPORTED", "only scalar int/bool/string file constants are linkable"));
        }
        let name = get_ident(&tokens, &mut i, pos)?;
        if tokens.get(i).is_some_and(|t| t.tok == Tok::Assign) || word(&tokens, i, "as") {
            i += 1;
        } else {
            return Err(fail(pos, "E_SYNTAX", "expected = or as in static constant"));
        }
        let start = i;
        while !tokens.get(i).is_some_and(|t| t.tok == Tok::Semi) {
            if tokens.get(i).is_none_or(|t| t.tok == Tok::Eof) {
                return Err(fail(pos, "E_EOF", "unterminated static constant"));
            }
            if i - start >= MAX_CONST_TOKENS {
                return Err(fail(pos, "E_SOURCE_LIMIT", "constant initializer exceeds token limit"));
            }
            i += 1;
        }
        let value = match (&ty[..], &tokens[start..i]) {
            ("int", [Token { tok: Tok::Int(Some(_)), .. }]) => {
                vec![tokens[start].tok.clone()]
            }
            ("int", [Token { tok: Tok::Minus, .. }, Token { tok: Tok::Int(_), .. }]) => {
                vec![tokens[start].tok.clone(), tokens[start + 1].tok.clone()]
            }
            ("bool", [Token { tok: Tok::Ident(s), .. }]) if s == "true" || s == "false" => {
                vec![tokens[start].tok.clone()]
            }
            ("string", [Token { tok: Tok::Str(_), .. }]) => {
                vec![tokens[start].tok.clone()]
            }
            (_, [Token { tok: Tok::Int(None), .. }]) => {
                return Err(fail(pos, "E_CONST_OVERFLOW", "constant i64 literal overflow"));
            }
            ("int" | "bool" | "string", [Token { tok: Tok::Int(_) | Tok::Str(_) | Tok::Float(_), .. }])
            | ("int" | "bool" | "string", [Token { tok: Tok::Ident(_), .. }]) => {
                return Err(fail(pos, "E_CONST_TYPE", "file constant type does not match literal initializer"));
            }
            _ => {
                // `std.env.read_env` and `os.read_env` require runtime
                // initialization, never compiler-machine environment folding.
                return Err(fail(pos, "E_UNSUPPORTED",
                    "non-literal or environment-backed file constants need typed runtime initialization"));
            }
        };
        i += 1; // ';'
        if exports.insert(name.clone(), ConstExport { public, value }).is_some() {
            return Err(fail(pos, "E_DUPLICATE", format!("duplicate file constant '{name}'")));
        }
        if exports.len() > MAX_BINDINGS {
            return Err(fail(pos, "E_SOURCE_LIMIT", "too many file constants"));
        }
    }
    Ok(exports)
}

fn is_disallowed_reference_context(tokens: &[Token], i: usize) -> Option<&'static str> {
    let before = i.checked_sub(1).and_then(|n| tokens.get(n));
    let after = tokens.get(i + 1);
    if after.is_some_and(|t| matches!(t.tok, Tok::Assign | Tok::PlusAssign | Tok::PlusPlus | Tok::MinusMinus))
        || before.is_some_and(|t| matches!(&t.tok, Tok::Ident(s)
            if matches!(s.as_str(), "mut" | "borrow" | "take" | "drop"))) {
        return Some("E_ASSIGN_CONST");
    }
    if after.is_some_and(|t| t.tok == Tok::LParen) {
        return Some("E_IMPORT_KIND");
    }
    if before.is_some_and(|t| t.tok == Tok::Dot)
        || after.is_some_and(|t| t.tok == Tok::Dot) {
        return Some("E_UNSUPPORTED");
    }
    if before.is_some_and(|t| matches!(&t.tok, Tok::Ident(s)
        if matches!(s.as_str(), "fnc" | "routine" | "val" | "let" | "const"
            | "class" | "actor" | "enum" | "int" | "bool" | "string"))) {
        return Some("E_DUPLICATE");
    }
    None
}

/// Rewrite a strictly bounded, const-import-only relative source unit.
/// `None` delegates all existing import forms to the established linker.
pub fn lower<F>(source: &str, logical: &Path, mut load: F) -> R<Option<Vec<Token>>>
where F: FnMut(&Path) -> std::io::Result<String> {
    if source.len() > crate::SOURCE_CAP {
        return Err(fail(Pos::default(), "E_SOURCE_LIMIT", "source exceeds byte limit"));
    }
    let tokens = lexer::lex(source)?;
    let has_typed = tokens.windows(2).any(|w|
        word(w, 0, "import") && word(w, 1, "const"));
    if !has_typed {
        return Ok(None);
    }
    reject_reserved(&tokens)?;
    if !word(&tokens, 0, "import") {
        return Err(fail(tokens[0].pos, "E_UNSUPPORTED",
            "executable constant file imports require import declarations first"));
    }
    let mut i = 0usize;
    let mut loaded = BTreeMap::<PathBuf, BTreeMap<String, ConstExport>>::new();
    let mut bound = BTreeMap::<String, Vec<Tok>>::new();
    while word(&tokens, i, "import") {
        let pos = tokens[i].pos;
        i += 1;
        if !word(&tokens, i, "const") {
            return Err(fail(pos, "E_UNSUPPORTED",
                "mixed const/function imports need the full typed linker"));
        }
        i += 1;
        let mut names = Vec::new();
        loop {
            if names.len() >= MAX_BINDINGS {
                return Err(fail(pos, "E_SOURCE_LIMIT", "too many constant selections"));
            }
            let remote = get_ident(&tokens, &mut i, pos)?;
            let local = if word(&tokens, i, "as") {
                i += 1;
                get_ident(&tokens, &mut i, pos)?
            } else { remote.clone() };
            names.push((remote, local));
            if tokens.get(i).is_some_and(|t| t.tok == Tok::Comma) { i += 1; }
            else { break; }
        }
        expect_word(&tokens, &mut i, "from", pos)?;
        if word(&tokens, i, "pkg") {
            return Err(fail(pos, "E_UNSUPPORTED",
                "named package constants need an authenticated package export resolver"));
        }
        let selector = match tokens.get(i).map(|t| &t.tok) {
            Some(Tok::Str(s)) => s,
            _ => return Err(fail(pos, "E_SYNTAX", "expected quoted constant file path")),
        };
        let path = source_path(logical, selector, pos)?;
        i += 1;
        if tokens.get(i).is_none_or(|t| t.tok != Tok::Semi) {
            return Err(fail(pos, "E_SYNTAX", "expected ';' after constant import"));
        }
        i += 1;
        if !loaded.contains_key(&path) {
            if loaded.len() >= MAX_FILES {
                return Err(fail(pos, "E_SOURCE_LIMIT", "too many imported files"));
            }
            let text = load(&path).map_err(|err| fail(pos, "E_IMPORT",
                format!("cannot load source file: {:?}", err.kind())))?;
            loaded.insert(path.clone(), file_constants(&text)?);
        }
        let exports = loaded.get(&path).expect("file inserted into bounded cache");
        for (remote, local) in names {
            if bound.contains_key(&local) {
                return Err(fail(pos, "E_DUPLICATE", format!("duplicate local import '{local}'")));
            }
            let item = exports.get(&remote).ok_or_else(|| fail(pos, "E_UNBOUND",
                format!("file has no constant '{remote}'")))?;
            if !item.public {
                return Err(fail(pos, "E_PRIVATE", format!("constant '{remote}' is not public")));
            }
            bound.insert(local, item.value.clone());
            if bound.len() > MAX_BINDINGS {
                return Err(fail(pos, "E_SOURCE_LIMIT", "too many imported constant bindings"));
            }
        }
    }
    let mut output = Vec::new();
    for (j, item) in tokens[i..].iter().enumerate() {
        if let Tok::Ident(name) = &item.tok {
            if let Some(value) = bound.get(name) {
                let absolute = i + j;
                if let Some(code) = is_disallowed_reference_context(&tokens, absolute) {
                    return Err(fail(item.pos, code,
                        format!("imported constant '{name}' used in disallowed context")));
                }
                for tok in value {
                    output.push(Token { tok: tok.clone(), pos: item.pos });
                }
                continue;
            }
        }
        output.push(item.clone());
    }
    if output.len() > lexer::MAX_TOKENS {
        return Err(fail(tokens[0].pos, "E_SOURCE_LIMIT", "linked token limit exceeded"));
    }
    Ok(Some(output))
}
