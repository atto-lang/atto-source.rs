//! A deliberately narrow, fail-closed first compiler profile for owned Dyn<I>.
//!
//! This is CLOSED-WORLD MONOMORPHIC devirtualization, not the general Dyn ABI:
//! exactly one local class may implement the interface. No vtable, type tag,
//! borrowed views, interface widening, storage fields, or tenant transport.
//! Preserves take! so the ordinary ownership checker diagnoses use after move.
//! Only interface methods and calls into a matching one-argument Dyn function
//! remain legal; erased values cannot be used as concrete class references.
//! An independent tagged runtime representation is required before supporting
//! more than one implementation. Never advertise this as polymorphic dispatch.
use std::collections::{BTreeMap, BTreeSet};

use crate::diag::{Diagnostic, Pos};
use crate::lexer::{Tok, Token};

type R<T> = Result<T, Diagnostic>;

fn word(t: &[Token], i: usize, s: &str) -> bool {
    matches!(t.get(i).map(|x| &x.tok), Some(Tok::Ident(n)) if n == s)
}
fn ident(t: &[Token], i: usize) -> Option<&str> {
    match t.get(i).map(|x| &x.tok) { Some(Tok::Ident(n)) => Some(n), _ => None }
}
fn err(p: Pos, code: &'static str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(p, code, message)
}
fn is(t: &[Token], i: usize, tok: Tok) -> bool {
    t.get(i).is_some_and(|x| x.tok == tok)
}
// The first executable Dyn profile is object-safe by construction: method
// parameters and results must be independent scalar values. A named object,
// Self, generic, borrow, ActorRef or nested container needs provenance-aware
// result lifetimes and cannot be passed through monomorphic erasure.
fn scalar(name: &str, allow_void: bool) -> bool {
    matches!(name, "int" | "bool" | "boolean" | "float" | "f64" | "string" | "String")
        || (allow_void && name == "void")
}
fn scalar_method_signature(t: &[Token], method: usize, semi: usize) -> bool {
    // Exact 'name(type arg, ...) : scalar ;' shape. The class/interface
    // conformance pass independently checks the complete named signature.
    if !is(t, method + 1, Tok::LParen) { return false; }
    let mut j = method + 2;
    while !is(t, j, Tok::RParen) {
        if j >= semi { return false; }
        let Some(ty) = ident(t, j) else { return false; };
        if !scalar(ty, false) || ident(t, j + 1).is_none() { return false; }
        j += 2;
        if is(t, j, Tok::RParen) { break; }
        if !is(t, j, Tok::Comma) { return false; }
        j += 1;
        if is(t, j, Tok::RParen) { return false; }
    }
    is(t, j + 1, Tok::Colon)
        && ident(t, j + 2).is_some_and(|ty| scalar(ty, true))
        && j + 3 == semi
}
#[derive(Clone)]
struct Contract {
    impls: Vec<String>,
    methods: BTreeSet<String>,
}
/// Source-local nominal signatures; executable method conformance is validated
/// separately by nominal_lower::lower, including exact public signatures.
fn contracts(t: &[Token]) -> R<BTreeMap<String, Contract>> {
    let mut result = BTreeMap::new();
    for i in 0..t.len() {
        if !word(t, i, "define") { continue; }
        let j = i + 1 + usize::from(word(t, i + 1, "pub"));
        if !word(t, j, "interface") { continue; }
        let Some(name) = ident(t, j + 1) else {
            return Err(err(t[i].pos, "E_DYN_INTERFACE", "interface is missing its name"));
        };
        if !word(t, j + 2, "as") {
            return Err(err(t[i].pos, "E_DYN_INTERFACE", "Dyn requires a source-local interface body"));
        }
        let mut methods = BTreeSet::new();
        let mut k = j + 3;
        while k < t.len() && !word(t, k, "end") {
            if word(t, k, "pub") { k += 1; continue; }
            let Some(method) = ident(t, k) else {
                return Err(err(t[k].pos, "E_DYN_INTERFACE", "unsupported Dyn interface member"));
            };
            if !is(t, k + 1, Tok::LParen) {
                return Err(err(t[k].pos, "E_DYN_INTERFACE", "Dyn interfaces require method signatures"));
            }
            if !methods.insert(method.to_owned()) {
                return Err(err(t[k].pos, "E_DUPLICATE", "duplicate interface method"));
            }
            let mut depth = 0usize;
            let mut end = k + 1;
            while end < t.len() {
                match t[end].tok {
                    Tok::LParen => depth += 1,
                    Tok::RParen => {
                        if depth == 0 { break; }
                        depth -= 1;
                    }
                    Tok::Semi if depth == 0 => break,
                    Tok::Eof | Tok::LBrace | Tok::RBrace if depth == 0 => break,
                    _ => {}
                }
                end += 1;
            }
            if !is(t, end, Tok::Semi) {
                return Err(err(t[k].pos, "E_DYN_INTERFACE",
                    "Dyn interface members must be abstract signatures ending ';'"));
            }
            if !scalar_method_signature(t, k, end) {
                return Err(err(t[k].pos, "E_DYN_ABI",
                    "browser Dyn<I> method ABI only admits scalar parameters and scalar/void results; no object, Self, generic or borrowed values"));
            }
            k = end + 1;
        }
        if !word(t, k, "end") {
            return Err(err(t[i].pos, "E_EOF", "unterminated Dyn interface"));
        }
        if result.insert(name.to_owned(), Contract { impls: Vec::new(), methods }).is_some() {
            return Err(err(t[i].pos, "E_DUPLICATE", "duplicate Dyn interface"));
        }
    }
    for i in 0..t.len() {
        if !word(t, i, "define") { continue; }
        let j = i + 1 + usize::from(word(t, i + 1, "pub"));
        if !word(t, j, "class") { continue; }
        let Some(class) = ident(t, j + 1) else { continue };
        let mut k = j + 2;
        while k < t.len() && !word(t, k, "as") && !is(t, k, Tok::Eof) {
            if word(t, k, "implements") {
                k += 1;
                loop {
                    let Some(interface) = ident(t, k) else { break };
                    if let Some(contract) = result.get_mut(interface) {
                        if !contract.impls.iter().any(|c| c == class) {
                            contract.impls.push(class.to_owned());
                        }
                    }
                    k += 1;
                    if !is(t, k, Tok::Comma) { break; }
                    k += 1;
                }
            } else {
                k += 1;
            }
            if k - j > 128 {
                return Err(err(t[i].pos, "E_SOURCE_LIMIT", "Dyn class header exceeds budget"));
            }
        }
    }
    Ok(result)
}

/// Return the matching interface for the exact four-token Dyn<I> spelling.
/// Generic nesting is intentionally unsupported in this first guest profile.
fn dyn_type<'a>(t: &'a [Token], i: usize) -> Option<&'a str> {
    if word(t, i, "Dyn") && is(t, i + 1, Tok::Lt)
        && is(t, i + 3, Tok::Gt) { ident(t, i + 2) }
    else { None }
}

/// Recognize only an explicitly typed local erasure:
///   val Dyn<I> erased = (take! source) as Dyn<I>;
///   val Dyn<I> erased = take! source as Dyn<I>;
/// No arbitrary casts, untyped destinations, borrowed sources, nested
/// erasures, or implicit conversion at a call site are admitted in v1.
/// Return the matching declaration's Dyn type token index on success.
fn owned_as_binder(t: &[Token], cast_at: usize, interface: &str) -> Option<usize> {
    if cast_at == 0 || !word(t, cast_at - 1, "as")
        || !is(t, cast_at + 4, Tok::Semi) { return None; }
    let binder = if cast_at >= 13
        && is(t, cast_at - 2, Tok::RParen)
        && ident(t, cast_at - 3).is_some()
        && is(t, cast_at - 4, Tok::Bang)
        && word(t, cast_at - 5, "take")
        && is(t, cast_at - 6, Tok::LParen)
        && is(t, cast_at - 7, Tok::Assign)
        && (word(t, cast_at - 13, "val") || word(t, cast_at - 13, "let")) {
        Some(cast_at - 12)
    } else if cast_at >= 11
        && ident(t, cast_at - 2).is_some()
        && is(t, cast_at - 3, Tok::Bang)
        && word(t, cast_at - 4, "take")
        && is(t, cast_at - 5, Tok::Assign)
        && (word(t, cast_at - 11, "val") || word(t, cast_at - 11, "let")) {
        Some(cast_at - 10)
    } else { None }?;
    if dyn_type(t, binder) != Some(interface)
        || ident(t, binder + 4).is_none()
        || !is(t, binder + 5, Tok::Assign) { return None; }
    Some(binder)
}

pub fn lower(t: Vec<Token>) -> R<Vec<Token>> {
    if !t.iter().any(|x| matches!(&x.tok, Tok::Ident(s) if s == "Dyn")) {
        return Ok(t);
    }
    let definitions = contracts(&t)?;
    let mut lowered_types = BTreeMap::<String, String>::new();
    for i in 0..t.len() {
        if !word(&t, i, "Dyn") { continue }
        let interface = dyn_type(&t, i).ok_or_else(||
            err(t[i].pos, "E_DYN_INTERFACE", "expected Dyn<Interface> with a single nominal interface"))?;
        let Some(def) = definitions.get(interface) else {
            return Err(err(t[i].pos, "E_DYN_INTERFACE",
                format!("Dyn<{interface}> requires a verified source-local interface")));
        };
        if def.impls.len() != 1 {
            return Err(err(t[i].pos, "E_DYN_RUNTIME", format!(
                "Dyn<{interface}> needs exactly one local implementing class for browser monomorphic lowering; general polymorphic runtime dispatch is not implemented"
            )));
        }
        lowered_types.insert(interface.to_owned(), def.impls[0].clone());
    }
    // Track every erased variable and the only permitted one-argument dynamic
    // interface entrypoints. The source type must remain an explicit Dyn<I>.
    let mut binders = BTreeMap::<String, (String, usize)>::new();
    let mut funcs = BTreeMap::<String, String>::new();
    let mut constructor_sites = BTreeSet::new();
    let mut cast_sites = BTreeSet::new();
    // Validate each 'as Dyn<I>' before erasing the interface tokens. The
    // source spelling is checked, not reconstructed from a concrete class.
    for i in 0..t.len() {
        if !word(&t, i, "as") || dyn_type(&t, i + 1).is_none() { continue; }
        let cast = i + 1;
        let interface = dyn_type(&t, cast).unwrap_or("");
        if owned_as_binder(&t, cast, interface).is_none() {
            return Err(err(t[i].pos, "E_DYN_OWNERSHIP",
                "as Dyn<I> requires a matching typed Dyn<I> local and (take! named_owner)"));
        }
        cast_sites.insert(cast);
    }
    for i in 0..t.len() {
        let Some(interface) = dyn_type(&t, i) else { continue };
        if cast_sites.contains(&i) { continue; }
        if is(&t, i + 4, Tok::LParen) {
            // Constructor syntax may be reduced ONLY to an explicit checked
            // take! of a named owner. The regular checker validates its class
            // type, loan state, and invalidates the moved source local.
            if !word(&t, i + 5, "take") || !is(&t, i + 6, Tok::Bang)
                || ident(&t, i + 7).is_none() || !is(&t, i + 8, Tok::RParen) {
                return Err(err(t[i].pos, "E_DYN_OWNERSHIP",
                    "owned Dyn constructor requires Dyn<I>(take! named_owner)"));
            }
            // The monomorphic carrier is a concrete class handle. Do not
            // let a Dyn constructor be used as an ordinary Greeting value:
            // that would silently reveal the implementation's extra methods.
            if i < 7 || dyn_type(&t, i - 6) != Some(interface)
                || !(word(&t, i - 7, "val") || word(&t, i - 7, "let"))
                || !is(&t, i - 1, Tok::Assign) {
                return Err(err(t[i].pos, "E_DYN_OWNERSHIP",
                    "owned Dyn construction requires a matching typed Dyn<I> local binding"));
            }
            constructor_sites.insert(i);
            continue;
        }
        let Some(var) = ident(&t, i + 4) else {
            return Err(err(t[i].pos, "E_DYN_RUNTIME",
                "Dyn return values, fields and composite types need a runtime dispatch ABI"));
        };
        let param = i >= 3 && is(&t, i - 1, Tok::LParen)
            && word(&t, i - 3, "fnc")
            && is(&t, i + 5, Tok::RParen);
        let bind = i >= 1 && (word(&t, i - 1, "val") || word(&t, i - 1, "let"))
            && is(&t, i + 5, Tok::Assign);
        if !param && !bind {
            return Err(err(t[i].pos, "E_DYN_RUNTIME",
                "browser owned Dyn supports only single-argument function parameters and local val/let bindings"));
        }
        if bind {
            let direct_take = word(&t, i + 6, "take") && is(&t, i + 7, Tok::Bang)
                && ident(&t, i + 8).is_some();
            let constructed = dyn_type(&t, i + 6) == Some(interface)
                && constructor_sites.contains(&(i + 6));
            // Constructor sites may appear later in this same token scan.
            let constructed = constructed || (dyn_type(&t, i + 6) == Some(interface)
                && is(&t, i + 10, Tok::LParen) && word(&t, i + 11, "take")
                && is(&t, i + 12, Tok::Bang) && ident(&t, i + 13).is_some()
                && is(&t, i + 14, Tok::RParen));
            let checked_as = cast_sites.contains(&(i + 12))
                || cast_sites.contains(&(i + 10));
            if !direct_take && !constructed && !checked_as {
                return Err(err(t[i].pos, "E_DYN_OWNERSHIP",
                    "Dyn local requires take! owner, Dyn<I>(take! owner), or (take! owner) as Dyn<I>"));
            }
        }
        if binders.insert(var.to_owned(), (interface.to_owned(), i + 4)).is_some() {
            return Err(err(t[i + 4].pos, "E_DYN_RUNTIME",
                "this browser Dyn profile requires unique erased variable names"));
        }
        if param {
            let function = ident(&t, i - 2).unwrap_or("");
            if funcs.insert(function.to_owned(), interface.to_owned()).is_some() {
                return Err(err(t[i].pos, "E_DYN_RUNTIME", "ambiguous Dyn function overload"));
            }
        }
    }
    // Validate every use before rewriting. With a monomorphic carrier type,
    // compiler type checking alone would otherwise allow private class APIs.
    for i in 0..t.len() {
        let Some(name) = ident(&t, i) else { continue };
        if let Some((interface, binder_at)) = binders.get(name) {
            if *binder_at == i { continue }
            if is(&t, i + 1, Tok::Dot) {
                let member = ident(&t, i + 2).unwrap_or("");
                if !definitions[interface].methods.contains(member)
                    || !is(&t, i + 3, Tok::LParen) {
                    return Err(err(t[i].pos, "E_DYN_INTERFACE", format!(
                        "Dyn<{interface}> exposes only its declared callable interface methods"
                    )));
                }
                continue;
            }
            if i >= 2 && is(&t, i - 1, Tok::LParen)
                && is(&t, i + 1, Tok::RParen)
                && ident(&t, i - 2).and_then(|callee| funcs.get(callee))
                    == Some(interface) {
                continue;
            }
            return Err(err(t[i].pos, "E_DYN_INTERFACE",
                "erased Dyn value cannot escape as a concrete class, be copied, cast or sent"));
        }
        if let Some(interface) = funcs.get(name) {
            if is(&t, i + 1, Tok::LParen) && !(i > 0 && word(&t, i - 1, "fnc")) {
                let actual = ident(&t, i + 2).and_then(|v| binders.get(v))
                    .filter(|(intf, _)| intf == interface);
                if !is(&t, i + 3, Tok::RParen) || actual.is_none() {
                    return Err(err(t[i].pos, "E_DYN_INTERFACE",
                        "Dyn interface function requires one explicitly erased Dyn argument"));
                }
            }
        }
    }
    let mut out = Vec::with_capacity(t.len());
    let mut i = 0;
    while i < t.len() {
        if word(&t, i, "as") && cast_sites.contains(&(i + 1)) {
            // Keep the original checked take! expression (and optional
            // parentheses), but remove only the proven redundant as Dyn<I>.
            i += 5;
        } else if let Some(interface) = dyn_type(&t, i) {
            if !constructor_sites.contains(&i) {
                out.push(Token { pos: t[i].pos,
                    tok: Tok::Ident(lowered_types[interface].clone()) });
            }
            // For an erasure constructor, retain the parenthesized take!
            // expression but not the Dyn type/function tokens.
            i += 4;
        } else {
            out.push(t[i].clone());
            i += 1;
        }
    }
    if out.len() > crate::lexer::MAX_TOKENS {
        return Err(err(t[0].pos, "E_SOURCE_LIMIT", "Dyn expansion exceeds token budget"));
    }
    Ok(out)
}
