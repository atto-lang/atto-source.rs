//! Bounded single-file export declarations and unlinked foreign prototypes.
//! This is not a package linker or a foreign-function runtime.
//! Public exports add deterministic Wasm metadata; private/package entries
//! cannot be imported as host functions.
use crate::diag::{Diagnostic, Pos};
use crate::lexer::{Tok, Token};
use crate::module_lower::ModuleDecl;
use std::collections::BTreeSet;
type R<T> = Result<T, Diagnostic>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Visibility {
    Public,
    Package,
}
#[derive(Clone, Debug)]
pub struct Export {
    pub name: String,
    pub kind: &'static str,
    pub visibility: Visibility,
    pub pos: Pos,
}
pub struct Lowered {
    pub tokens: Vec<Token>,
    pub exports: Vec<Export>,
}
fn word(t: &[Token], i: usize, value: &str) -> bool {
    matches!(t.get(i).map(|x| &x.tok), Some(Tok::Ident(s)) if s == value)
}
fn error(pos: Pos, code: &'static str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(pos, code, message)
}
fn name(t: &[Token], i: usize) -> R<String> {
    match t.get(i) {
        Some(Token {
            tok: Tok::Ident(n),
            pos: _,
        }) if !n.is_empty()
            && !matches!(
                n.as_str(),
                "pub" | "export" | "class" | "fnc" | "module" | "namespace" | "as" | "end"
            ) =>
        {
            Ok(n.clone())
        }
        Some(x) => Err(error(x.pos, "E_SYNTAX", "expected bounded export identifier")),
        None => Err(error(Pos { line: 1, col: 1 }, "E_EOF", "missing export name")),
    }
}
/// Export ABI admits canonical ASCII identifiers of at most 256 bytes.
fn export_name(t: &[Token], i: usize) -> R<String> {
    let id = name(t, i)?;
    let pos = t[i].pos;
    if !id.is_ascii() {
        return Err(error(pos, "E_EXPORT_NAME", "export identifier must be ASCII"));
    }
    if id.len() > 256 {
        return Err(error(pos, "E_EXPORT_NAME_TOO_LONG", "export identifier exceeds 256 bytes"));
    }
    Ok(id)
}
/// One parser for file-level export directives, shared between the normal
/// pre-lowering pipeline and direct AST parser. The binder is still responsible
/// for symbol ownership and declaration-kind validation.
pub fn parse_export_directive(t: &[Token], start: usize) -> R<(Export, usize)> {
    let pos = t.get(start).map(|x| x.pos).unwrap_or(Pos { line: 1, col: 1 });
    if !word(t, start, "export") || !word(t, start + 1, "pub") {
        return Err(error(pos, "E_SYNTAX", "export requires pub or pub[pkg]"));
    }
    let mut j = start + 2;
    let visibility = if t.get(j).map(|x| &x.tok) == Some(&Tok::LBracket) {
        if !word(t, j + 1, "pkg") || t.get(j + 2).map(|x| &x.tok) != Some(&Tok::RBracket) {
            return Err(error(pos, "E_SYNTAX", "only pub[pkg] package visibility is supported"));
        }
        j += 3;
        Visibility::Package
    } else {
        Visibility::Public
    };
    let mut kind = "";
    if ["class", "fnc", "module", "actor", "enum"].iter().any(|k| word(t, j, k)) {
        kind = if word(t, j, "class") {
            "class"
        } else if word(t, j, "module") {
            "module"
        } else if word(t, j, "actor") {
            "actor"
        } else if word(t, j, "enum") {
            "enum"
        } else {
            "fnc"
        };
        j += 1;
    }
    let id = export_name(t, j)?;
    if t.get(j + 1).map(|x| &x.tok) != Some(&Tok::Semi) {
        return Err(error(pos, "E_SYNTAX", "export must end with semicolon"));
    }
    Ok((
        Export {
            name: id,
            kind,
            visibility,
            pos,
        },
        j + 2,
    ))
}

/// Resolve exports against the checked parser's actual declaration table,
/// not arbitrary tokens: a class member named Foo is NOT a top-level fnc
/// Foo, and a foreign prototype never grants source-level export authority.
pub fn resolve_exports(exports: &mut [Export], parsed: &crate::ast::Program, modules: &[ModuleDecl]) -> R<()> {
    for export in exports {
        let class_count = parsed
            .classes
            .iter()
            .filter(|c| c.name == export.name && !c.state_container)
            .count();
        // A function export selects the whole checked overload set, not one
        // declaration. Duplicate signatures are rejected by the typechecker.
        let fnc_count = parsed.funcs.iter().filter(|f| f.name == export.name).count();
        let module = modules.iter().find(|m| m.name == export.name);
        let actor_count = parsed.actors.iter().filter(|a| a.name == export.name).count();
        let enum_count = parsed.enums.iter().filter(|e| e.name == export.name).count();
        let kind_count = usize::from(class_count > 0)
            + usize::from(fnc_count > 0)
            + usize::from(module.is_some())
            + usize::from(actor_count > 0)
            + usize::from(enum_count > 0);
        if kind_count == 0 {
            return Err(error(
                export.pos,
                "E_UNBOUND",
                format!("export '{}' has no file-local declaration", export.name),
            ));
        }
        if kind_count > 1 || class_count > 1 || actor_count > 1 || enum_count > 1 {
            return Err(error(
                export.pos,
                "E_DUPLICATE",
                format!("export '{}' resolves to ambiguous declaration kinds", export.name),
            ));
        }
        let kind = if class_count > 0 {
            "class"
        } else if fnc_count > 0 {
            "fnc"
        } else if actor_count > 0 {
            "actor"
        } else if enum_count > 0 {
            "enum"
        } else {
            "module"
        };
        if !export.kind.is_empty() && export.kind != kind {
            return Err(error(export.pos, "E_EXPORT_KIND", "export declaration kind mismatch"));
        }
        if module.is_some_and(|m| m.inline_public) && export.visibility == Visibility::Package {
            return Err(error(
                export.pos,
                "E_VISIBILITY",
                "public module declaration conflicts with package-only export",
            ));
        }
        export.kind = kind;
    }
    Ok(())
}
/// Resolve direct-parser or pre-lowered directives into one checked AST
/// export table. No metadata can originate from an unresolved source export.
pub fn bind_program_exports(program: &mut crate::ast::Program, mut lexical: Vec<Export>, modules: &[ModuleDecl]) -> R<()> {
    if !program.exports.is_empty() {
        return Err(error(Pos::default(), "E_DUPLICATE", "program already has a resolved export table"));
    }
    lexical.extend(std::mem::take(&mut program.parsed_exports));
    if lexical.len() > 128 {
        return Err(error(Pos::default(), "E_SOURCE_LIMIT", "export budget exceeded"));
    }
    let mut seen = BTreeSet::new();
    for e in &lexical {
        if !seen.insert(e.name.clone()) {
            return Err(error(e.pos, "E_DUPLICATE", format!("duplicate exported name '{}'", e.name)));
        }
    }
    // Inline public modules synthesize one export, while a matching explicit
    // directive is validated for kind and visibility conflicts.
    for module in modules {
        if module.inline_public && !seen.contains(&module.name) {
            if lexical.len() >= 128 {
                return Err(error(module.pos, "E_SOURCE_LIMIT", "export budget exceeded"));
            }
            lexical.push(Export {
                name: module.name.clone(),
                kind: "module",
                visibility: Visibility::Public,
                pos: module.pos,
            });
        }
    }
    resolve_exports(&mut lexical, program, modules)?;
    lexical.sort_by(|a, b| a.name.cmp(&b.name));
    for export in &lexical {
        if export.kind == "class" && export.visibility == Visibility::Public {
            if let Some(class) = program
                .classes
                .iter_mut()
                .find(|class| class.name == export.name && !class.state_container)
            {
                class.public = true;
            }
        }
    }
    program.exports = lexical
        .into_iter()
        .map(|e| {
            let kind = match e.kind {
                "class" => crate::ast::ExportKind::Class,
                "fnc" => crate::ast::ExportKind::Fnc,
                "actor" => crate::ast::ExportKind::Actor,
                "enum" => crate::ast::ExportKind::Enum,
                "module" => crate::ast::ExportKind::Module,
                _ => return Err(error(e.pos, "E_EXPORT_KIND", "unresolved or unsupported export kind")),
            };
            Ok(crate::ast::ExportDecl {
                name: e.name,
                pos: e.pos,
                kind,
                visibility: match e.visibility {
                    Visibility::Public => crate::ast::ExportVisibility::Public,
                    Visibility::Package => crate::ast::ExportVisibility::Package,
                },
            })
        })
        .collect::<R<Vec<_>>>()?;
    Ok(())
}

fn external_prototype(t: &[Token], start: usize) -> R<(usize, String)> {
    let p = t[start].pos;
    if !word(t, start + 1, "fnc") {
        return Err(error(p, "E_UNSUPPORTED", "only unlinked external fnc prototypes are recognized"));
    }
    let id = name(t, start + 2)?;
    let mut i = start + 3;
    if t.get(i).map(|x| &x.tok) != Some(&Tok::LParen) {
        return Err(error(p, "E_SYNTAX", "external fnc requires parameter list"));
    }
    let mut level = 0usize;
    loop {
        match t.get(i).map(|x| &x.tok) {
            Some(Tok::LParen) => level += 1,
            Some(Tok::RParen) => {
                if level == 0 {
                    return Err(error(p, "E_SYNTAX", "invalid external parentheses"));
                }
                level -= 1;
                if level == 0 {
                    i += 1;
                    break;
                }
            }
            Some(Tok::Eof) | None => return Err(error(p, "E_EOF", "unclosed external prototype")),
            _ => {}
        }
        if level > 32 {
            return Err(error(p, "E_NESTING", "external signature nesting limit"));
        }
        i += 1;
    }
    if !matches!(t.get(i).map(|x| &x.tok), Some(Tok::Colon | Tok::Arrow)) {
        return Err(error(p, "E_SYNTAX", "external prototype needs result annotation"));
    }
    i += 1;
    if !matches!(t.get(i).map(|x| &x.tok), Some(Tok::Ident(_))) {
        return Err(error(p, "E_SYNTAX", "external prototype needs a result type"));
    }
    i += 1;
    if t.get(i).map(|x| &x.tok) != Some(&Tok::Semi) {
        return Err(error(
            p,
            "E_UNSUPPORTED",
            "external definitions and foreign ABI calls remain unsupported",
        ));
    }
    Ok((i + 1, id))
}
/// Export directives are *file-level*. Removing nested tokens before the
/// parser sees them could accidentally publish another scope's declaration.
fn reject_nested_exports(t: &[Token]) -> R<()> {
    let mut braces = 0usize;
    // 'as ... end' nominal bodies, ':: ... end' routines and match/select
    // expressions must not make a nested export appear file-local.
    let mut end_scopes = 0usize;
    for i in 0..t.len() {
        if (word(t, i, "export") || word(t, i, "internal") || word(t, i, "external")) && (braces > 0 || end_scopes > 0) {
            return Err(error(
                t[i].pos,
                "E_SYNTAX",
                "export, internal and external declarations are only allowed at file scope",
            ));
        }
        match &t[i].tok {
            Tok::LBrace => braces += 1,
            Tok::RBrace => braces = braces.saturating_sub(1),
            // Arrow-return callables may use an implicit or explicit
            // 'end' body even WITHOUT '::'. Count that body separately from
            // the surrounding class/module so its first 'end' cannot cause
            // an inner export to masquerade as file scope.
            Tok::Arrow if braces == 0 => {
                // A return annotation can be followed by effect clauses
                // before its braced body. Do not mistake such a function
                // for an end-delimited one and poison later top-level exports.
                let mut j = i + 2;
                while ["throws", "raises", "panics"].iter().any(|k| word(t, j, k)) && t.get(j + 1).map(|x| &x.tok) == Some(&Tok::LParen) {
                    j += 2;
                    while j < t.len() && !matches!(t[j].tok, Tok::RParen | Tok::Eof) {
                        j += 1;
                    }
                    if t.get(j).map(|x| &x.tok) != Some(&Tok::RParen) {
                        break;
                    }
                    j += 1;
                    if t.get(j).map(|x| &x.tok) == Some(&Tok::Comma) {
                        j += 1;
                    }
                }
                let body_start = t.get(j).map(|x| &x.tok);
                if body_start != Some(&Tok::LBrace) && body_start != Some(&Tok::ColonColon) {
                    end_scopes += 1;
                }
            }
            Tok::ColonColon if braces == 0 => end_scopes += 1,
            Tok::Ident(s) if braces == 0 => {
                if s == "define" {
                    let kind_index = if word(t, i + 1, "pub") { i + 2 } else { i + 1 };
                    if ["class", "actor", "enum", "module"].iter().any(|k| word(t, kind_index, k)) {
                        end_scopes += 1;
                    }
                } else if matches!(s.as_str(), "match" | "switch" | "select") {
                    end_scopes += 1;
                } else if s == "end" {
                    end_scopes = end_scopes.saturating_sub(1);
                }
            }
            _ => {}
        }
    }
    Ok(())
}

pub fn lower(t: Vec<Token>) -> R<Lowered> {
    if !t.iter().any(|x| {
        matches!(&x.tok, Tok::Ident(s)
        if matches!(s.as_str(), "export" | "external" | "internal" | "protected"))
    }) {
        return Ok(Lowered {
            tokens: t,
            exports: vec![],
        });
    }
    reject_nested_exports(&t)?;
    let mut exports: Vec<Export> = Vec::new();
    let mut foreign = BTreeSet::new();
    let mut internal = BTreeSet::new();
    let mut out = Vec::new();
    let mut i = 0;
    while i < t.len() {
        let pos = t[i].pos;
        if word(&t, i, "export") {
            if exports.len() >= 128 {
                return Err(error(pos, "E_SOURCE_LIMIT", "export budget exceeded"));
            }
            let (export, next) = parse_export_directive(&t, i)?;
            if exports.iter().any(|e| e.name == export.name) {
                return Err(error(pos, "E_DUPLICATE", format!("duplicate exported name '{}'", export.name)));
            }
            exports.push(export);
            i = next;
            continue;
        }
        if word(&t, i, "external") {
            if i > 0 && word(&t, i - 1, "pub") {
                return Err(error(pos, "E_VISIBILITY", "external prototype cannot be declared pub"));
            }
            let (next, foreign_name) = external_prototype(&t, i)?;
            if !foreign.insert(foreign_name) {
                return Err(error(pos, "E_DUPLICATE", "duplicate external prototype"));
            }
            i = next;
            continue;
        }
        if word(&t, i, "internal") {
            if (i > 0 && word(&t, i - 1, "pub")) || word(&t, i + 1, "pub") {
                return Err(error(pos, "E_VISIBILITY", "internal function cannot also be pub"));
            }
            if !word(&t, i + 1, "fnc") {
                return Err(error(pos, "E_UNSUPPORTED", "only source-file local internal fnc is admitted"));
            }
            let id = name(&t, i + 2)?;
            // Internal overloads share one private symbol identity; their
            // arities/signatures are checked by the ordinary type checker.
            internal.insert(id);
            i += 1;
            continue;
        }
        if word(&t, i, "protected") {
            return Err(error(
                pos,
                "E_UNSUPPORTED",
                "protected requires checked subclass visibility; it cannot be lowered as private",
            ));
        }
        out.push(t[i].clone());
        i += 1;
    }
    for export in &exports {
        if internal.contains(&export.name) {
            return Err(error(
                export.pos,
                "E_VISIBILITY",
                format!("internal function '{}' cannot be exported", export.name),
            ));
        }
    }
    for x in &out {
        if let Tok::Ident(n) = &x.tok {
            if foreign.contains(n) {
                return Err(error(
                    x.pos,
                    "E_UNSUPPORTED",
                    format!("foreign prototype '{n}' cannot be referenced without a validated ABI"),
                ));
            }
        }
    }
    // Export-name resolution is deferred to resolve_exports after parsing.
    // A token sweep cannot prove source-level class/function membership.
    Ok(Lowered { tokens: out, exports })
}
