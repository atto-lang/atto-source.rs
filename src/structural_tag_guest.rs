//! Source-local marker/brand admission for the existing scalar snapshot ABI.
//! Deliberately only two flat functions, one named structural formal, and a
//! direct immutable class-local call. No tag values, imports, inheritance,
//! generic constraints, dynamic patterns, or guest metadata. The ordinary
//! parser, scalar projection lowerer, checker and emitter still own codegen.
use std::collections::BTreeMap;
use crate::ast::{BindKind, ExprKind, FieldMode, Program, Stmt, Type};
use crate::diag::{Diagnostic, Pos};
use crate::lexer::{Tok, Token};
use crate::parser::Parser;
use crate::structural_tags::{ModuleId, TagError, TagId, TagKind, TagRegistry, TagTarget, Visibility};

type R<T> = Result<T, Diagnostic>;

fn unsupported(pos: Pos, why: &str) -> Diagnostic {
    Diagnostic::new(pos, "E_TAG_UNSUPPORTED", why)
}

fn tag_diag(pos: Pos, error: TagError) -> Diagnostic {
    let code = match error {
        TagError::MissingBrand => "E_MISSING_BRAND",
        TagError::MissingMarker => "E_MISSING_MARKER",
        _ => "E_TAG_DECLARATION",
    };
    Diagnostic::new(pos, code, format!("static marker/brand proof: {error:?}"))
}

fn word(tokens: &[Token], i: usize, name: &str) -> bool {
    matches!(tokens.get(i).map(|t| &t.tok), Some(Tok::Ident(w)) if w == name)
}

fn at(tokens: &[Token], i: usize, tok: &Tok) -> bool {
    tokens.get(i).is_some_and(|t| &t.tok == tok)
}

fn pos(tokens: &[Token], i: usize) -> Pos {
    tokens.get(i).map_or_else(Pos::default, |t| t.pos)
}

fn tag_kind(tokens: &[Token], i: usize) -> Option<TagKind> {
    if word(tokens, i, "marker") { Some(TagKind::Marker) }
    else if word(tokens, i, "brand") { Some(TagKind::Brand) }
    else { None }
}

fn tag_name(tokens: &[Token], i: usize) -> R<&str> {
    match tokens.get(i).map(|t| &t.tok) {
        Some(Tok::Ident(w)) if w.starts_with('#') && w.len() > 1 => Ok(w),
        _ => Err(unsupported(pos(tokens, i), "expected a #tag declaration reference")),
    }
}

fn class_name(tokens: &[Token], i: usize) -> R<&str> {
    match tokens.get(i).map(|t| &t.tok) {
        Some(Tok::Ident(w)) if !w.starts_with('#') => Ok(w),
        _ => Err(unsupported(pos(tokens, i), "expected a class name")),
    }
}

fn tagged_token(t: &Token) -> bool {
    matches!(&t.tok, Tok::Ident(w) if w.starts_with('#'))
}

/// The tag-aware lexer is private to compile(). Every # token is consumed
/// here in a declaration/header position or rejected before ordinary parsing.
/// IDs are local to this one closed compilation; neither registry nor IDs
/// escape it, so no cross-module/process/epoch identity is implied.
pub(crate) fn parse_if_tagged(tokens: &[Token]) -> R<Option<Program>> {
    let Some(first) = tokens.iter().find(|t| tagged_token(t)) else { return Ok(None); };
    let start = first.pos;
    let mut registry = TagRegistry::new(1, 1).map_err(|e| tag_diag(start, e))?;
    let owner = ModuleId::new(1, 1).map_err(|e| tag_diag(start, e))?;
    let mut declared = BTreeMap::<String, TagId>::new();
    let mut classes = BTreeMap::<String, (u32, Vec<TagId>)>::new();
    let mut i = 0;
    let mut clean = Vec::with_capacity(tokens.len());
    // Initial tag declarations must be contiguous before any classes.
    while word(tokens, i, "define") {
        let public = word(tokens, i + 1, "pub");
        let k = i + 1 + usize::from(public);
        let Some(kind) = tag_kind(tokens, k) else { break; };
        let name = tag_name(tokens, k + 1)?;
        if !at(tokens, k + 2, &Tok::Semi) {
            return Err(unsupported(pos(tokens, k + 2), "tag declaration requires ';'"));
        }
        if declared.contains_key(name) {
            return Err(Diagnostic::new(pos(tokens, k + 1), "E_TAG_DECLARATION", "duplicate tag declaration"));
        }
        let id = registry.declare(owner, (declared.len() + 1) as u64, kind,
            if public { Visibility::Public } else { Visibility::Private })
            .map_err(|e| tag_diag(pos(tokens, k + 1), e))?;
        declared.insert(name.to_owned(), id);
        i = k + 3;
    }
    if declared.is_empty() {
        return Err(Diagnostic::new(start, "E_UNSUPPORTED", "Symbols are not supported; tag declarations must precede classes"));
    }
    // Only remove well-formed attachment clauses in top-level class headers.
    // Preserve the original position-bearing tokens for the production parser.
    loop {
        let begin = i;
        let d = i + usize::from(word(tokens, i, "pub"));
        if !word(tokens, d, "define") || !word(tokens, d + 1, "class") { break; }
        let name = class_name(tokens, d + 2)?.to_owned();
        if classes.contains_key(&name) {
            return Err(Diagnostic::new(pos(tokens, d + 2), "E_TAG_DECLARATION", "duplicate class declaration"));
        }
        let id = (classes.len() + 1) as u32;
        clean.extend_from_slice(&tokens[begin..d + 3]);
        i = d + 3;
        let mut attached = Vec::new();
        while word(tokens, i, "with") {
            let kind = tag_kind(tokens, i + 1).ok_or_else(|| unsupported(pos(tokens, i + 1), "expected marker or brand clause"))?;
            i += 2;
            loop {
                let name = tag_name(tokens, i)?;
                let tag = *declared.get(name).ok_or_else(|| tag_diag(pos(tokens, i), TagError::UnknownTag))?;
                attached.push(registry.resolve(owner, tag, kind).map_err(|e| tag_diag(pos(tokens, i), e))?);
                i += 1;
                if !at(tokens, i, &Tok::Comma) { break; }
                i += 1;
            }
        }
        if !word(tokens, i, "as") {
            return Err(unsupported(pos(tokens, i), "only flat class declarations with 'as' are admitted"));
        }
        registry.register_flat_type(id, 1, owner, &attached).map_err(|e| tag_diag(pos(tokens, d + 2), e))?;
        classes.insert(name, (id, attached));
        // Flat profile only: subsequent AST validation rejects methods,
        // constructors, state and nonliteral initializers; no inner tag
        // token may ever be stripped or passed to the normal parser.
        while !word(tokens, i, "end") {
            let t = tokens.get(i).ok_or_else(|| unsupported(start, "unterminated class"))?;
            if tagged_token(t) || t.tok == Tok::Eof {
                return Err(unsupported(t.pos, "tag value or unterminated class"));
            }
            clean.push(t.clone());
            i += 1;
        }
        clean.push(tokens[i].clone());
        i += 1;
    }
    if classes.is_empty() || classes.values().all(|(_, tags)| tags.is_empty()) {
        return Err(unsupported(start, "expected at least one tagged flat class"));
    }
    if tokens[i..].iter().any(tagged_token) {
        return Err(unsupported(start, "tag references outside a flat class header require checked lowering"));
    }
    clean.extend_from_slice(&tokens[i..]);
    let program = Parser::new(clean).parse_program()?;
    // Every class parsed by the production parser must be one of the
    // declarations registered above. Never pass a tagged AST into an
    // unrelated lowering that could discard its obligations.
    if program.classes.len() != classes.len() || !program.actors.is_empty()
        || !program.enums.is_empty() || program.actor_interface.is_some()
        || program.funcs.len() != 2 {
        return Err(unsupported(start, "tagged scalar profile needs flat classes and exactly two functions"));
    }
    for c in &program.classes {
        if !classes.contains_key(&c.name) || c.state_container || c.ctor.is_some()
            || !c.methods.is_empty() || c.fields.is_empty() || c.fields.len() > 8
            || c.fields.iter().any(|f| !f.public || f.ty != Type::Int
                || f.mode == FieldMode::ActorState
                || !matches!(&f.init.kind, ExprKind::Int(Some(_)))) {
            return Err(unsupported(c.pos, "only ordinary literal public-int class fields are admitted"));
        }
    }
    let main = program.funcs.iter().find(|f| f.name == "main")
        .ok_or_else(|| unsupported(start, "missing main function"))?;
    let read = program.funcs.iter().find(|f| f.name != "main")
        .ok_or_else(|| unsupported(start, "missing structural function"))?;
    if main.is_generator || read.is_generator || !main.params.is_empty()
        || main.ret != Type::Int || read.ret != Type::Int {
        return Err(unsupported(start, "only synchronous int functions are admitted"));
    }
    let [formal] = read.params.as_slice() else {
        return Err(unsupported(read.pos, "exactly one structural formal required"));
    };
    if !formal.structural || formal.mutable {
        return Err(unsupported(formal.pos, "readonly structural class formal required"));
    }
    let Type::Named(target_index) = formal.ty else {
        return Err(unsupported(formal.pos, "named structural class required"));
    };
    let (target, _) = program.type_names.get(target_index as usize)
        .ok_or_else(|| unsupported(formal.pos, "unresolved structural class"))?;
    let (target_id, _) = classes.get(target)
        .ok_or_else(|| unsupported(formal.pos, "target must be a registered class"))?;
    let [Stmt::Return(Some(result), _)] = read.body.as_slice() else {
        return Err(unsupported(read.pos, "only a direct structural field return is admitted"));
    };
    if !matches!(&result.kind, ExprKind::Member(base, _) if matches!(&base.kind, ExprKind::Var(n) if n == &formal.name)) {
        return Err(unsupported(result.pos, "structural receiver cannot escape or be used dynamically"));
    }
    let [Stmt::Bind { kind: BindKind::Val, name, ty: None, init, .. },
        Stmt::Return(Some(call), _)] = main.body.as_slice() else {
        return Err(unsupported(main.pos, "main must bind one immutable inferred class local and return its structural call"));
    };
    let ExprKind::New(actual, ctor_args) = &init.kind else {
        return Err(unsupported(init.pos, "actual source must be a new ordinary class"));
    };
    if !ctor_args.is_empty() {
        return Err(unsupported(init.pos, "custom construction requires a separate brand invariant proof"));
    }
    let (actual_id, _) = classes.get(actual)
        .ok_or_else(|| unsupported(init.pos, "actual source must be a registered class"))?;
    let ExprKind::Call(callee, args) = &call.kind else {
        return Err(unsupported(call.pos, "direct structural call required"));
    };
    if callee != &read.name || !matches!(args.as_slice(), [a] if matches!(&a.kind, ExprKind::Var(v) if v == name)) {
        return Err(unsupported(call.pos, "exactly one checked call on the immutable source local required"));
    }
    registry.check_static_tags(owner, *actual_id, 1,
        TagTarget::Named { nominal_id: *target_id, layout_epoch: 1, additional: &[] })
        .map_err(|e| tag_diag(call.pos, e))?;
    // Independently require the existing scalar projection pass to succeed,
    // never rely on the tag conjunct to authorize access or change layout.
    // The normal check::check path repeats that lowering on the original AST.
    let lowered = crate::structural_call_lower::lower(&program)?;
    if lowered.funcs.iter().flat_map(|f| &f.params).any(|p| p.structural) {
        return Err(unsupported(call.pos, "existing scalar projection ABI did not admit the call"));
    }
    Ok(Some(program))
}
