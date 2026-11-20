//! Fail-closed admission for unimplemented AttoLang scoped Mutex acquisition.
//!
//! This is a deliberately conservative *negative* gate, not a Mutex compiler,
//! a borrow checker, a guest scheduler, or evidence of runtime support.
//! Typed ownership/lease lowering must replace it before admission. The lexer
//! has already stripped comments and strings, so preview text cannot spoof it.
use crate::diag::Diagnostic;
use crate::lexer::{Tok, Token};

/// Refuse the recognizable `using [await] mutex.lock_async()`/lock() profile
/// whenever a built-in Mutex/SharedMutex/MutexLease type is present. Without
/// this gate, overlapping parser or syntax-only backends can accidentally
/// turn the scope into an ordinary call and emit unsafe executable Wasm.
pub fn reject_unimplemented(tokens: &[Token]) -> Result<(), Diagnostic> {
    let typed_mutex = tokens.windows(2).any(|pair| {
        matches!(
            (&pair[0].tok, &pair[1].tok),
            (Tok::Ident(name), Tok::Lt | Tok::LParen)
                if matches!(name.as_str(), "Mutex" | "SharedMutex" | "MutexLease")
        )
    });
    if !typed_mutex {
        return Ok(());
    }
    let lock_call = tokens.windows(3).any(|triple| {
        matches!(
            (&triple[0].tok, &triple[1].tok, &triple[2].tok),
            (Tok::Dot, Tok::Ident(method), Tok::LParen)
                if matches!(method.as_str(), "lock_async" | "lock" | "try_lock")
        )
    });
    if !lock_call {
        return Ok(());
    }
    if let Some(scope) = tokens.iter().find(|token| {
        matches!(&token.tok, Tok::Ident(word) if word == "using")
    }) {
        return Err(Diagnostic::new(
            scope.pos,
            "E_LOCK_UNSUPPORTED",
            "scoped Mutex acquisition requires verified typed lease/await lowering,              bounded nonparking waiters and cleanup; compiler guest emission is disabled",
        ));
    }
    Ok(())
}
