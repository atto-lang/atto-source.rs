//! Fail-closed source preflight for proposed unary actor reply signatures.
//!
//! Stage 1 ONLY: parses a bounded, non-executable signature. It does not prove
//! nominal UnaryRequest<R>, Sendable, suspends, turn completion or reply ABI.
//! The browser compiler MUST reject this source rather than emit fake Wasm.
use crate::diag::{Diagnostic, Pos};
use crate::lexer::{Tok, Token};

type R<T> = Result<T, Diagnostic>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnarySignature {
    pub actor: String,
    pub request: String,
    pub response: String,
    pub http: bool,
    pub suspends: bool,
    pub pos: Pos,
}

fn is(toks: &[Token], i: usize, word: &str) -> bool {
    matches!(toks.get(i).map(|t| &t.tok), Some(Tok::Ident(w)) if w == word)
}
fn tok(toks: &[Token], i: usize, t: &Tok) -> bool {
    toks.get(i).is_some_and(|x| x.tok == *t)
}
fn pos(toks: &[Token], i: usize) -> Pos {
    toks.get(i).map(|t| t.pos).unwrap_or_default()
}
fn err(toks: &[Token], i: usize, code: &'static str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(pos(toks, i), code, message)
}
fn name(toks: &[Token], i: &mut usize, what: &str) -> R<String> {
    if let Some(Token { tok: Tok::Ident(s), .. }) = toks.get(*i) {
        if matches!(s.as_str(), "any" | "Any") {
            return Err(err(toks, *i, "E_REPLY_TYPE",
                format!("{what} cannot use erased any without a proven nominal reply contract")));
        }
        let result = s.clone();
        *i += 1;
        Ok(result)
    } else {
        Err(err(toks, *i, "E_SYNTAX", format!("expected {what}")))
    }
}
fn expect(toks: &[Token], i: &mut usize, t: Tok, what: &str) -> R<()> {
    if tok(toks, *i, &t) { *i += 1; Ok(()) }
    else { Err(err(toks, *i, "E_SYNTAX", format!("expected {what}"))) }
}
fn expect_word(toks: &[Token], i: &mut usize, w: &str) -> R<()> {
    if is(toks, *i, w) { *i += 1; Ok(()) }
    else { Err(err(toks, *i, "E_SYNTAX", format!("expected '{w}'"))) }
}

fn unary_at(toks: &[Token], start: usize) -> R<UnarySignature> {
    let mut i = start;
    expect_word(toks, &mut i, "define")?;
    expect_word(toks, &mut i, "unary")?;
    let http = if is(toks, i, "http") { i += 1; true } else { false };
    expect_word(toks, &mut i, "actor")?;
    let actor = name(toks, &mut i, "actor name")?;
    let mut declared_request: Option<String> = None;
    let mut declared_emits = false;
    let mut declared_sends: Option<String> = None;
    let mut after_output = false;
    while is(toks, i, "receives") || is(toks, i, "emits") || is(toks, i, "sends") {
        if is(toks, i, "receives") {
            if declared_request.is_some() || after_output {
                return Err(err(toks, i, "E_REPLY_SIGNATURE",
                    "duplicate or out-of-order receives clause"));
            }
            i += 1;
            declared_request = Some(name(toks, &mut i, "unary request type")?);
        } else if is(toks, i, "emits") {
            if declared_emits {
                return Err(err(toks, i, "E_REPLY_SIGNATURE", "duplicate emits clause"));
            }
            // Pipeline emits is independent of unary replies. It is accepted
            // as a header role but does not satisfy a response obligation.
            after_output = true;
            declared_emits = true;
            i += 1;
            let _ = name(toks, &mut i, "pipeline item type")?;
        } else {
            if declared_sends.is_some() {
                return Err(err(toks, i, "E_REPLY_SIGNATURE", "duplicate sends clause"));
            }
            after_output = true;
            i += 1;
            declared_sends = Some(name(toks, &mut i, "application send type")?);
        }
    }
    let request = declared_request.ok_or_else(|| err(toks, i, "E_REPLY_SIGNATURE",
        "unary actors must declare receives M where M implements UnaryRequest<R>"))?;
    expect_word(toks, &mut i, "as")?;
    let suspends = if is(toks, i, "susp") || is(toks, i, "suspends") {
        i += 1;
        true
    } else { false };
    expect_word(toks, &mut i, "receive")?;
    expect(toks, &mut i, Tok::LParen, "'(' after receive")?;
    expect_word(toks, &mut i, "ActorMail")?;
    expect(toks, &mut i, Tok::Lt, "'<' after ActorMail")?;
    let mail_request = name(toks, &mut i, "ActorMail request type")?;
    expect(toks, &mut i, Tok::Gt, "'>' after ActorMail type")?;
    let mail_var = name(toks, &mut i, "mail parameter")?;
    expect(toks, &mut i, Tok::RParen, "')' after mail parameter")?;
    if request != mail_request {
        return Err(err(toks, i, "E_REPLY_TYPE",
            format!("actor receives {request} but sealed ActorMail payload is {mail_request}")));
    }
    if !(tok(toks, i, &Tok::Colon) || tok(toks, i, &Tok::Arrow)) {
        return Err(err(toks, i, "E_REPLY_SIGNATURE",
            "unary receive requires an explicit non-void response return type"));
    }
    i += 1;
    let response = name(toks, &mut i, "unary response type")?;
    if response == "void" {
        return Err(err(toks, i.saturating_sub(1), "E_REPLY_TYPE",
            "unary receive cannot return void; use a unit-valued UnaryRequest<Unit>"));
    }
    // This is a conservative preliminary sends check for a single nominal
    // output. Rich union/multi-type effects require the real type checker.
    if let Some(send) = &declared_sends {
        if send != &response {
            return Err(err(toks, i, "E_REPLY_TYPE",
                format!("declared sends {send} cannot cover unary reply type {response} in the single-type profile")));
        }
    }
    expect(toks, &mut i, Tok::LBrace, "'{' after unary receive signature")?;
    let mut depth = 1usize;
    let mut has_return = false;
    let mut explicit_reply = false;
    // Bounded by the lexer MAX_TOKENS. No AST body/effect claim is made.
    while depth != 0 {
        let Some(next) = toks.get(i) else {
            return Err(err(toks, i, "E_EOF", "unclosed unary receive body"));
        };
        match &next.tok {
            Tok::LBrace => { depth += 1; if depth > 128 {
                return Err(err(toks, i, "E_NESTING", "unary handler nesting exceeds 128"));
            } },
            Tok::RBrace => depth -= 1,
            Tok::Eof => return Err(err(toks, i, "E_EOF", "unclosed unary receive body")),
            Tok::Ident(w) if w == "return" => {
                if tok(toks, i + 1, &Tok::Semi) || tok(toks, i + 1, &Tok::RBrace) {
                    return Err(err(toks, i, "E_REPLY_TYPE",
                        "unary return-value handler cannot use bare return; expected a response value"));
                }
                has_return = true;
            },
            Tok::Dot if is(toks, i + 1, "respond_with") ||
                        is(toks, i + 1, "send_back") => {
                // This bans mixing the implicit return reply with explicit
                // respond_with until a true path-sensitive ownership checker
                // can establish mutually exclusive behavior.
                if i > 0 && is(toks, i - 1, &mail_var) { explicit_reply = true; }
            }
            _ => {}
        }
        i += 1;
    }
    if explicit_reply {
        return Err(err(toks, start, "E_REPLY_LINEAR",
            "a unary return-value handler cannot also use m.respond_with/send_back"));
    }
    if !has_return {
        return Err(err(toks, start, "E_REPLY_MISSING",
            "unary return-value handler has no return expression"));
    }
    expect_word(toks, &mut i, "end")?;
    // No additional actor members are admitted in this first scanner slice.
    // A future complete frontend must parse all members and check all paths,
    // not silently skip unsupported code.
    Ok(UnarySignature { actor, request, response, http, suspends, pos: pos(toks, start) })
}

/// Inspect proposed unary headers; absence leaves legacy source unchanged.
/// Successful inspection is NOT permission to generate guest code.
pub fn inspect(toks: &[Token]) -> R<Vec<UnarySignature>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        if is(toks, i, "define") && is(toks, i + 1, "unary") {
            if out.len() >= 128 {
                return Err(err(toks, i, "E_SOURCE_LIMIT", "too many unary actor declarations"));
            }
            out.push(unary_at(toks, i)?);
        }
        i += 1;
    }
    Ok(out)
}

/// This profile is never executable. Ordinary ActorMail response sugar must
/// also be blocked until the runtime supplies an authenticated reply slot.
pub fn uses_reply_intrinsic(toks: &[Token]) -> bool {
    // Only an authenticated receive(ActorMail<T> mail) parameter grants
    // reply capability. Do not hijack unrelated user-defined methods named
    // respond_with/reply_stream/send_back on other classes.
    let mut mail_names: Vec<&str> = Vec::new();
    for i in 0..toks.len() {
        if !is(toks, i, "receive") || !tok(toks, i + 1, &Tok::LParen) ||
            !is(toks, i + 2, "ActorMail") || !tok(toks, i + 3, &Tok::Lt) ||
            !tok(toks, i + 5, &Tok::Gt) || !tok(toks, i + 7, &Tok::RParen) {
            continue;
        }
        if let (Some(Token { tok: Tok::Ident(_), .. }),
                Some(Token { tok: Tok::Ident(mail), .. })) =
            (toks.get(i + 4), toks.get(i + 6)) {
            mail_names.push(mail);
        }
    }
    toks.windows(3).any(|w| {
        let Tok::Ident(receiver) = &w[0].tok else { return false };
        if !mail_names.contains(&receiver.as_str()) || w[1].tok != Tok::Dot {
            return false;
        }
        matches!(&w[2].tok, Tok::Ident(name) if
            matches!(name.as_str(), "respond_with" | "send_back" | "reply_stream"))
    })
}
