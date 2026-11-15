//! Actor-state cleanup obligations for the typed receiver frontend.
//!
//! This constructs source-located, bounded cleanup contracts for sealed
//! `self.handle_state` callbacks. The callback is syntactically checked by
//! target_syntax and state field types are checked by target_typed. No backend
//! consumes these contracts to emit lease unlocks yet; every obligation is
//! explicitly unproved for runtime execution.
use crate::diag::{Diagnostic, Pos};
use crate::lexer::{Tok, Token};
use crate::target_syntax::ReceiverBodyTokens;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StateLeaseMode { SharedRead, ExclusiveWrite }

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LeaseCleanupExit { Normal, CheckedPanic }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeaseCleanupObligation {
    pub lease: usize,
    pub exit: LeaseCleanupExit,
    pub pos: Pos,
    /// The release is required by semantics but has not been emitted or
    /// verified in the native, browser Wasm or other language backends.
    pub backend_release_proven: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateLeaseCleanupRegion {
    pub lease: usize,
    pub receiver: usize,
    pub mode: StateLeaseMode,
    pub acquire_pos: Pos,
    pub normal_end_pos: Pos,
    pub checked_arithmetic: bool,
    /// Release obligations must remain explicit even for read-only callbacks.
    pub cleanup: Vec<LeaseCleanupObligation>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReceiverCleanupPlan {
    pub receiver: usize,
    pub leases: Vec<StateLeaseCleanupRegion>,
    /// This is intentionally false whenever executable semantics are not
    /// established. A syntactically balanced callback is not a runtime proof.
    pub backend_cleanup_proven: bool,
}

fn word(toks: &[Token], i: usize, value: &str) -> bool {
    matches!(toks.get(i).map(|t| &t.tok),
        Some(Tok::Ident(name)) if name == value)
}
fn is(toks: &[Token], i: usize, tok: &Tok) -> bool {
    toks.get(i).is_some_and(|t| &t.tok == tok)
}
fn err(toks: &[Token], i: usize, code: &'static str,
    message: &'static str) -> Diagnostic {
    Diagnostic::new(toks.get(i).map_or(Pos::default(), |t| t.pos), code, message)
}
/// A strictly bounded and bracket-matching delimiter scan. No unbalanced or
/// mismatched callback may generate a pretend cleanup obligation.
fn close_paren(toks: &[Token], open: usize) -> Result<usize, Diagnostic> {
    if !is(toks, open, &Tok::LParen) {
        return Err(err(toks,open,"E_SYNTAX","expected state lease callback '('"));
    }
    let mut expected=vec![Tok::RParen];
    let mut idx=open+1;
    while idx<toks.len() {
        match &toks[idx].tok {
            Tok::LParen => expected.push(Tok::RParen),
            Tok::LBracket => expected.push(Tok::RBracket),
            Tok::LBrace => expected.push(Tok::RBrace),
            Tok::RParen | Tok::RBracket | Tok::RBrace => {
                if expected.pop().as_ref() != Some(&toks[idx].tok) {
                    return Err(err(toks,idx,"E_SYNTAX",
                        "mismatched state lease callback delimiter"));
                }
                if expected.is_empty() { return Ok(idx); }
            }
            _ => {}
        }
        if expected.len()>128 {
            return Err(err(toks,idx,"E_NESTING",
                "state lease callback nesting limit"));
        }
        idx+=1;
    }
    Err(err(toks,open,"E_SYNTAX","unclosed state lease callback"))
}

fn checked_trap(tok: &Tok) -> bool {
    matches!(tok,Tok::Plus | Tok::Minus | Tok::Star | Tok::Slash |
        Tok::PlusAssign | Tok::PlusPlus | Tok::MinusMinus)
}

fn callback_region(toks:&[Token], receiver:usize, open:usize, end:usize,
    lease:usize) -> Result<StateLeaseCleanupRegion,Diagnostic> {
    // These positions are validated independently by target_syntax; refuse
    // to build a cleanup plan for a fabricated or malformed token layout.
    if !word(toks,open+1,"critical") || !is(toks,open+2,&Tok::Minus)
        || !(word(toks,open+3,"read")||word(toks,open+3,"write"))
        || !is(toks,open+4,&Tok::Pipe)
        || !matches!(toks.get(open+5).map(|t|&t.tok),Some(Tok::Ident(_)))
        || !is(toks,open+6,&Tok::Pipe)
        || !is(toks,open+7,&Tok::Arrow)
        || end<=open+8 {
        return Err(err(toks,open,"E_SEALED",
            "state cleanup requires a validated critical callback"));
    }
    let mode=if word(toks,open+3,"write") {
        StateLeaseMode::ExclusiveWrite
    } else { StateLeaseMode::SharedRead };
    let content=&toks[open+8..end];
    // Loops/branches have unproved state transitions, including implicit
    // reentrancy and paths which might skip the lexical callback ending.
    // Until cleanup lowering handles every edge, never admit them under a
    // state lease. Existing primitive scalar assignments still work.
    for t in content {
        if let Tok::Ident(name)=&t.tok {
            if ["if","else","elif","while","for","loop","do","match",
                "switch","select","defer","try","catch","finally",
                "return","break","continue","throw","raise","panic",
                "await","yield","spawn","cancel"].contains(&name.as_str()) {
                return Err(Diagnostic::new(t.pos,"E_BORROW",
                    "branching, suspension, cancellation or early exit under an actor state lease needs proven runtime cleanup"));
            }
        }
        if matches!(t.tok,Tok::Question | Tok::YieldStar) {
            return Err(Diagnostic::new(t.pos,"E_BORROW",
                "implicit exit under an actor state lease needs proven runtime cleanup"));
        }
    }
    let checked_arithmetic=content.iter().any(|t|checked_trap(&t.tok));
    let normal_end_pos=toks[end].pos;
    let mut cleanup=vec![LeaseCleanupObligation {
        lease,exit:LeaseCleanupExit::Normal,pos:normal_end_pos,
        backend_release_proven:false,
    }];
    if checked_arithmetic {
        // This edge cannot be treated as a normal return: a checked integer
        // arithmetic trap may unwind while the read/write lease is held.
        cleanup.push(LeaseCleanupObligation {
            lease,exit:LeaseCleanupExit::CheckedPanic,
            pos:content.iter().find(|t|checked_trap(&t.tok))
                .map_or(normal_end_pos,|t|t.pos),
            backend_release_proven:false,
        });
    }
    Ok(StateLeaseCleanupRegion {
        lease,receiver,mode,
        acquire_pos:toks[open-3].pos,normal_end_pos,
        checked_arithmetic,cleanup,
    })
}

/// One plan per receiver body. A normal-release obligation is always present
/// for each callback. Checked arithmetic adds a separate panic cleanup edge.
/// Any unsupported callback control flow is a compile-time error, not a
/// runtime cleanup proof or a claim that destructors were generated.
pub fn plan_receiver_cleanup(body:&ReceiverBodyTokens)
    -> Result<ReceiverCleanupPlan,Diagnostic> {
    let toks=&body.tokens;
    let mut plan=ReceiverCleanupPlan {
        receiver:body.receiver,leases:Vec::new(),backend_cleanup_proven:false,
    };
    let mut i=0;
    while i+3<toks.len() {
        if word(toks,i,"self") && is(toks,i+1,&Tok::Dot)
            && word(toks,i+2,"handle_state")
            && is(toks,i+3,&Tok::LParen) {
            let end=close_paren(toks,i+3)?;
            let lease=plan.leases.len();
            plan.leases.push(callback_region(toks,body.receiver,i+3,end,lease)?);
            // The existing syntax checker has already prohibited nested
            // handle_state calls; skip this callback as one acquisition.
            i=end+1;
        } else {i+=1;}
    }
    Ok(plan)
}

/// Executable lowerers must not silently infer cleanup safety from token or
/// typed-frontend facts. This remains a hard gate until a backend consumes
/// each cleanup edge and verifies its emitted release on all relevant exits.
pub fn require_backend_cleanup_proof(plan:&ReceiverCleanupPlan)
    -> Result<(),Diagnostic> {
    // A receiver with no state leases has no state-lease unlock obligations.
    // The independent typed body, ownership and AOT runtime gates remain
    // mandatory; this does not grant executable permission by itself.
    if plan.leases.is_empty() { return Ok(()); }
    if !plan.backend_cleanup_proven ||
        plan.leases.iter().flat_map(|l|&l.cleanup)
            .any(|edge|!edge.backend_release_proven) {
        return Err(Diagnostic::new(
            plan.leases.first().map_or(Pos::default(),|l|l.acquire_pos),
            "E_UNSUPPORTED",
            "executable actor receiver cleanup has no verified backend proof"));
    }
    Ok(())
}
