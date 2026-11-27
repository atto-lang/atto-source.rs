//! Bounded nonallocating Pointer<int> local-place specialization.
//!
//! This is a compiler lowering, not a raw-pointer interpreter or an allocation
//! ABI. The *production lexer* supplies tokens; the verifier recognizes only
//! one straight-line source-local scalar place, one readonly handle, one copy
//! read, exactly one consuming release and the scalar return. It then erases
//! the nonescaping handle and sends ordinary scalar source tokens through the
//! canonical production parser, type checker and real Wasm generator.
//!
//! Other Pointer<T> programs fail closed. In particular this does not borrow
//! the ownership-moving CheckedRef<Class> semantics, does not grant write or
//! close authority, and cannot extend local references across calls, actors,
//! exceptions, branches, loops, threads, FFI, or suspension.
//!
//! Conformance: atto-spec#18 and atto-platform-conformance PRL-001/002.
//! This subset is *not* a general typed Pointer<T> HIR/MIR implementation.

use crate::diag::{Diagnostic, Pos};
use crate::lexer::{Tok, Token};

type R<T> = Result<T, Diagnostic>;

fn is_word(t: &Tok, name: &str) -> bool {
    matches!(t, Tok::Ident(word) if word == name)
}

fn failure(pos: Pos, code: &'static str, message: &str) -> Diagnostic {
    Diagnostic::new(pos, code, message)
}

struct Cursor<'a> {
    tokens: &'a [Token],
    at: usize,
}

impl Cursor<'_> {
    fn token(&self) -> &Token {
        // The lexer always appends Eof. The caller admits only lexed input.
        &self.tokens[self.at.min(self.tokens.len() - 1)]
    }

    fn eat(&mut self, wanted: Tok, code: &'static str) -> R<Token> {
        let token = self.token().clone();
        if token.tok != wanted {
            return Err(failure(token.pos, code, "unsupported Pointer<int> local-place expression or control-flow profile"));
        }
        self.at += 1;
        Ok(token)
    }

    fn word(&mut self, expected: &str, code: &'static str) -> R<Token> {
        let token = self.token().clone();
        if !is_word(&token.tok, expected) {
            return Err(failure(token.pos, code, "unsupported Pointer<int> local-place declaration, effect or lifetime"));
        }
        self.at += 1;
        Ok(token)
    }

    fn ident(&mut self) -> R<Token> {
        let t = self.token().clone();
        let Tok::Ident(ref name) = t.tok else {
            return Err(failure(t.pos, "E_POINTER_PROFILE", "expected a named local place or checked handle"));
        };
        let valid = name.starts_with(|c: char| c.is_ascii_lowercase() || c == '_')
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && !matches!(name.as_str(), "val" | "let" | "const" | "rt" |
                "ref" | "deref" | "unref" | "Pointer" | "int" |
                "pub" | "fnc" | "main" | "return" | "mut" |
                "drop" | "copy" | "share");
        if !valid || crate::parser::is_reserved(name) || matches!(name.as_str(), "null" | "to" | "empty" | "true" | "false") {
            return Err(failure(t.pos, "E_POINTER_PROFILE", "unsupported local identifier in bounded pointer profile"));
        }
        self.at += 1;
        Ok(t)
    }

    fn name_of(t: &Token) -> &str {
        if let Tok::Ident(ref name) = t.tok { name } else { unreachable!() }
    }

    // A grouping paren is never an allocation, pointer cast, or expression.
    fn place(&mut self) -> R<Token> {
        if self.token().tok == Tok::LParen {
            self.eat(Tok::LParen, "E_POINTER_PLACE")?;
            let result = self.ident()?;
            self.eat(Tok::RParen, "E_POINTER_PLACE")?;
            Ok(result)
        } else {
            self.ident()
        }
    }

    fn operation(&mut self, op: &str) -> R<Pos> {
        let first = self.token().clone();
        if is_word(&first.tok, "rt") {
            self.word("rt", "E_POINTER_SYNTAX")?;
            self.word(op, "E_POINTER_SYNTAX")?;
            return Ok(first.pos);
        }
        self.word(op, "E_POINTER_SYNTAX")?;
        let bang = self.eat(Tok::Bang, "E_POINTER_SYNTAX")?;
        if bang.pos.line != first.pos.line || bang.pos.col != first.pos.col + op.len() as u32 {
            return Err(failure(bang.pos, "E_POINTER_SYNTAX", "ownership bang must be adjacent to the operator"));
        }
        // Reject glued operands before any token erasure. The production lexer
        // already enforces its own whitespace rules; token positions alone
        // cannot authenticate the contents of a skipped comment.
        let next = self.token();
        if next.pos.line == bang.pos.line && next.pos.col <= bang.pos.col + 1 {
            return Err(failure(next.pos, "E_POINTER_SYNTAX", "ownership bang requires an operand separator"));
        }
        Ok(first.pos)
    }
}

/// The presence of a val Pointer<...> declaration requests this *explicitly
/// bounded* compiler profile. All nonmatching programs take the existing
/// production pipeline, including its fail-closed generic pointer gate.
/// No source-text substring search can accidentally admit code in comments.
pub fn lower(tokens: Vec<Token>) -> R<Vec<Token>> {
    if tokens.len() < 2 {
        return Ok(tokens);
    }
    let requests = tokens.windows(3).any(|w| {
        is_word(&w[0].tok, "val") && is_word(&w[1].tok, "Pointer") && w[2].tok == Tok::Lt
    });
    if !requests {
        return Ok(tokens);
    }

    let mut c = Cursor { tokens: &tokens, at: 0 };
    // No imports, source-local functions, callbacks, actors, effects, or
    // alternate entry points: the backend can prove *every* exit is release.
    c.word("pub", "E_POINTER_PROFILE")?;
    c.word("fnc", "E_POINTER_PROFILE")?;
    c.word("main", "E_POINTER_PROFILE")?;
    c.eat(Tok::LParen, "E_POINTER_PROFILE")?;
    c.eat(Tok::RParen, "E_POINTER_PROFILE")?;
    c.eat(Tok::Colon, "E_POINTER_PROFILE")?;
    c.word("int", "E_POINTER_TYPE")?;
    c.eat(Tok::LBrace, "E_POINTER_PROFILE")?;
    let head_end = c.at;

    // The referent must be a stable, initialized, addressable int local.
    let owner_start = c.at;
    c.word("let", "E_POINTER_PLACE")?;
    let place = c.ident()?;
    c.eat(Tok::Colon, "E_POINTER_PLACE")?;
    c.word("int", "E_POINTER_TYPE")?;
    c.eat(Tok::Assign, "E_POINTER_PLACE")?;
    if !matches!(c.token().tok, Tok::Int(Some(_))) {
        return Err(failure(c.token().pos, "E_POINTER_UNINITIALIZED",
            "local place must have a statically valid initialized int"));
    }
    c.at += 1;
    c.eat(Tok::Semi, "E_POINTER_PLACE")?;
    let owner_end = c.at;

    c.word("val", "E_POINTER_PROFILE")?;
    c.word("Pointer", "E_POINTER_TYPE")?;
    c.eat(Tok::Lt, "E_POINTER_TYPE")?;
    c.word("int", "E_POINTER_TYPE")?;
    c.eat(Tok::Gt, "E_POINTER_TYPE")?;
    let handle = c.ident()?;
    if Cursor::name_of(&handle) == Cursor::name_of(&place) {
        return Err(failure(handle.pos, "E_POINTER_ALIAS", "checked handle cannot shadow its referent"));
    }
    c.eat(Tok::Assign, "E_POINTER_PROFILE")?;
    c.operation("ref")?;
    let source = c.place()?;
    if Cursor::name_of(&source) != Cursor::name_of(&place) {
        return Err(failure(source.pos, "E_POINTER_PLACE", "reference must target the initialized local place"));
    }
    c.eat(Tok::Semi, "E_POINTER_PROFILE")?;

    let answer_start = c.at;
    c.word("val", "E_POINTER_PROFILE")?;
    let result = c.ident()?;
    if Cursor::name_of(&result) == Cursor::name_of(&place)
        || Cursor::name_of(&result) == Cursor::name_of(&handle) {
        return Err(failure(result.pos, "E_POINTER_ALIAS", "result must not shadow live owner or handle"));
    }
    c.eat(Tok::Assign, "E_POINTER_PROFILE")?;
    let rhs_pos = c.token().pos;
    c.operation("deref")?;
    let read_handle = c.place()?;
    if Cursor::name_of(&read_handle) != Cursor::name_of(&handle) {
        return Err(failure(read_handle.pos, "E_POINTER_AUTHORITY", "deref requires the authenticated local handle"));
    }
    c.eat(Tok::Semi, "E_POINTER_PROFILE")?;
    let answer_end = c.at;

    c.operation("unref")?;
    let released = c.place()?;
    if Cursor::name_of(&released) != Cursor::name_of(&handle) {
        return Err(failure(released.pos, "E_POINTER_AUTHORITY", "unref must consume the same live local handle"));
    }
    c.eat(Tok::Semi, "E_POINTER_LIFETIME")?;

    // The only control-flow exit occurs *after* the release. No other use
    // or copy of either handle or referent is possible in this profile.
    let return_start = c.at;
    c.word("return", "E_POINTER_LIFETIME")?;
    let returned = c.ident()?;
    if Cursor::name_of(&returned) != Cursor::name_of(&result) {
        return Err(failure(returned.pos, "E_POINTER_ESCAPE", "return only the copied integer, never the handle"));
    }
    c.eat(Tok::Semi, "E_POINTER_LIFETIME")?;
    c.eat(Tok::RBrace, "E_POINTER_LIFETIME")?;
    c.eat(Tok::Eof, "E_POINTER_LIFETIME")?;
    if c.at != tokens.len() {
        return Err(failure(c.token().pos, "E_POINTER_ESCAPE",
            "pointer-local profile cannot contain additional source declarations"));
    }

    // Nonallocating proven stack promotion: the pointer exists only as a
    // statically verified lexical access path and never as guest bytes.
    // Feed the scalar borrow-copy operation through the normal compiler.
    let mut lowered = Vec::with_capacity(head_end + owner_end - owner_start + 9);
    lowered.extend_from_slice(&tokens[..head_end]);
    lowered.extend_from_slice(&tokens[owner_start..owner_end]);
    lowered.extend_from_slice(&tokens[answer_start..answer_start + 3]); // val result =
    lowered.push(Token { tok: Tok::Ident(Cursor::name_of(&place).to_string()), pos: rhs_pos });
    lowered.push(tokens[answer_end - 1].clone()); // ;
    lowered.extend_from_slice(&tokens[return_start..]);
    Ok(lowered)
}


/// Candidate reusable RawPointer<int> profile (atto-spec #287).
/// Only closed, straight-line main, literal scalar owners, one non-owning
/// mutable slot, explicit bind/read/detach and scalar return. No address
/// bytes, allocation, ownership transfer, alias publication or class ABI.
/// This specialization proves the slot is unobservable and erases it;
/// unsupported programs fail closed, including every RawPointer<T> outside
/// this grammar. Existing linear Pointer<T> lowering stays independent.
pub fn lower_slot(tokens: Vec<Token>) -> R<Vec<Token>> {
    if !tokens.iter().any(|t| is_word(&t.tok, "RawPointer")) {
        return Ok(tokens);
    }
    const PROFILE: &str = "E_POINTER_SLOT_PROFILE";
    const LIMIT: usize = 32;
    let mut c = Cursor { tokens: &tokens, at: 0 };
    c.word("pub", PROFILE)?;
    c.word("fnc", PROFILE)?;
    c.word("main", PROFILE)?;
    c.eat(Tok::LParen, PROFILE)?;
    c.eat(Tok::RParen, PROFILE)?;
    c.eat(Tok::Colon, PROFILE)?;
    c.word("int", "E_POINTER_TYPE")?;
    c.eat(Tok::LBrace, PROFILE)?;
    let mut lowered = tokens[..c.at].to_vec();
    let mut owners = std::collections::BTreeSet::<String>::new();
    let mut values = std::collections::BTreeSet::<String>::new();

    // Owners are distinct, initialized, scalar locals. No later writes or
    // moves are admitted, so copied reads can safely refer to the same place.
    while is_word(&c.token().tok, "let")
        && !c.tokens.get(c.at + 1).is_some_and(|t| is_word(&t.tok, "mut"))
    {
        let start = c.at;
        c.word("let", PROFILE)?;
        let owner = c.ident()?;
        let name = Cursor::name_of(&owner).to_owned();
        if owners.len() >= LIMIT || !owners.insert(name.clone()) {
            return Err(failure(owner.pos, "E_POINTER_ALIAS", "duplicate owner or owner limit"));
        }
        values.insert(name);
        c.eat(Tok::Colon, PROFILE)?;
        c.word("int", "E_POINTER_TYPE")?;
        c.eat(Tok::Assign, PROFILE)?;
        if !matches!(c.token().tok, Tok::Int(Some(_))) {
            return Err(failure(c.token().pos, "E_POINTER_UNINITIALIZED", "owner must be an initialized integer literal"));
        }
        c.at += 1;
        c.eat(Tok::Semi, PROFILE)?;
        lowered.extend_from_slice(&tokens[start..c.at]);
    }
    c.word("let", PROFILE)?;
    c.word("mut", PROFILE)?;
    c.word("RawPointer", PROFILE)?;
    c.eat(Tok::Lt, PROFILE)?;
    c.word("int", "E_POINTER_TYPE")?;
    c.eat(Tok::Gt, PROFILE)?;
    let slot = c.ident()?;
    let slot_name = Cursor::name_of(&slot);
    if values.contains(slot_name) || crate::parser::is_reserved(slot_name)
        || matches!(slot_name, "null" | "to" | "empty" | "true" | "false") {
        return Err(failure(slot.pos, "E_POINTER_ALIAS", "slot cannot shadow a value or reserved name"));
    }
    c.eat(Tok::Assign, PROFILE)?;
    c.word("RawPointer", PROFILE)?;
    c.eat(Tok::Dot, PROFILE)?;
    c.word("empty", PROFILE)?;
    c.eat(Tok::Lt, PROFILE)?;
    c.word("int", "E_POINTER_TYPE")?;
    c.eat(Tok::Gt, PROFILE)?;
    c.eat(Tok::LParen, PROFILE)?;
    c.eat(Tok::RParen, PROFILE)?;
    c.eat(Tok::Semi, PROFILE)?;
    let mut bound: Option<String> = None;
    let mut operations = 0usize;
    while !is_word(&c.token().tok, "return") {
        operations += 1;
        if operations > LIMIT {
            return Err(failure(c.token().pos, PROFILE, "pointer slot operation limit"));
        }
        if is_word(&c.token().tok, "ref") {
            let pos = c.operation("ref")?;
            if bound.is_some() {
                return Err(failure(pos, "E_POINTER_LIFETIME", "detach bound slot before rebinding"));
            }
            let handle = c.ident()?;
            if Cursor::name_of(&handle) != slot_name {
                return Err(failure(handle.pos, "E_POINTER_AUTHORITY", "unknown pointer slot"));
            }
            c.word("to", PROFILE)?;
            let source = c.ident()?;
            let name = Cursor::name_of(&source);
            if !owners.contains(name) {
                return Err(failure(source.pos, "E_POINTER_PLACE", "bind requires an initialized local owner"));
            }
            c.eat(Tok::Semi, PROFILE)?;
            bound = Some(name.to_owned());
        } else if is_word(&c.token().tok, "val") {
            let start = c.at;
            c.word("val", PROFILE)?;
            let result = c.ident()?;
            let name = Cursor::name_of(&result);
            if name == slot_name || !values.insert(name.to_owned()) {
                return Err(failure(result.pos, "E_POINTER_ALIAS", "read result cannot shadow a live name"));
            }
            c.eat(Tok::Assign, PROFILE)?;
            let pos = c.token().pos;
            // Reuse the adjacent-bang check, without granting rt aliases.
            if !is_word(&c.token().tok, "deref") {
                return Err(failure(pos, PROFILE, "slot reads require deref!"));
            }
            c.operation("deref")?;
            let handle = c.ident()?;
            if Cursor::name_of(&handle) != slot_name {
                return Err(failure(handle.pos, "E_POINTER_AUTHORITY", "unknown pointer slot"));
            }
            let owner = bound.as_ref().ok_or_else(|| failure(pos, "E_POINTER_LIFETIME", "cannot dereference empty slot"))?;
            let semi = c.eat(Tok::Semi, PROFILE)?;
            lowered.extend_from_slice(&tokens[start..start + 3]);
            lowered.push(Token { tok: Tok::Ident(owner.clone()), pos });
            lowered.push(semi);
        } else if is_word(&c.token().tok, "unref") {
            let pos = c.operation("unref")?;
            let handle = c.ident()?;
            if Cursor::name_of(&handle) != slot_name {
                return Err(failure(handle.pos, "E_POINTER_AUTHORITY", "unknown pointer slot"));
            }
            if bound.is_none() {
                return Err(failure(pos, "E_POINTER_LIFETIME", "cannot detach empty slot"));
            }
            c.eat(Tok::Semi, PROFILE)?;
            bound = None;
        } else {
            return Err(failure(c.token().pos, PROFILE, "only explicit bind, copied read and detach are admitted; null assignment is forbidden"));
        }
    }
    if bound.is_some() {
        return Err(failure(c.token().pos, "E_POINTER_LIFETIME", "live slot on return"));
    }
    let start = c.at;
    c.word("return", PROFILE)?;
    // Only verified scalar names or literals may escape; the slot never can.
    // A short + / - chain is left to the ordinary checked integer compiler.
    for index in 0..LIMIT {
        if matches!(c.token().tok, Tok::Int(Some(_))) {
            c.at += 1;
        } else {
            let scalar = c.ident()?;
            if !values.contains(Cursor::name_of(&scalar)) {
                return Err(failure(scalar.pos, "E_POINTER_ESCAPE", "return must use verified scalar values"));
            }
        }
        if !matches!(c.token().tok, Tok::Plus | Tok::Minus) {
            break;
        }
        if index + 1 == LIMIT {
            return Err(failure(c.token().pos, PROFILE, "return expression limit"));
        }
        c.at += 1;
    }
    c.eat(Tok::Semi, PROFILE)?;
    c.eat(Tok::RBrace, PROFILE)?;
    c.eat(Tok::Eof, PROFILE)?;
    if c.at != tokens.len() {
        return Err(failure(c.token().pos, "E_POINTER_ESCAPE", "additional declarations are not admitted"));
    }
    lowered.extend_from_slice(&tokens[start..]);
    Ok(lowered)
}
