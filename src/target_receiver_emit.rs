//! Restricted, *experimental* receiver instruction emission from validated
//! Atto source. This is deliberately not the production actor/Wasm backend.
//! Neither these instructions nor their Rust execution witness authorize mtd
//! compilation: the independent E_UNSUPPORTED gate in lib.rs stays closed.
use crate::diag::{Diagnostic, Pos};
use crate::lexer::{lex, Tok};
use crate::target_cleanup::StateLeaseMode;
use crate::target_cleanup_cfg::{
    lower_verified_cleanup_cfg, CleanupExit, VerifiedCleanupCfg,
};
use crate::target_syntax::parse_target_declarations;
use crate::target_typed::{lower_target_signatures, TargetType};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReceiverInstruction {
    Acquire { lease: usize, mode: StateLeaseMode },
    Load { lease: usize },
    CheckedAddArgument { lease: usize },
    CheckedAddLiteral { lease: usize, value: i64 },
    Store { lease: usize },
    Release { lease: usize },
    Exit(CleanupExit),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmittedExitPath {
    pub exit: CleanupExit,
    pub pos: Pos,
    pub instructions: Vec<ReceiverInstruction>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReceiverKind {
    Read,
    ReadChecked(i64),
    AddArgument,
}

/// Private provenance: only the source-to-instruction lowerer builds this.
/// The native executor below is a Rust test witness, not an Atto runtime.
#[derive(Clone, Debug)]
pub struct ExperimentalReceiver {
    kind: ReceiverKind,
    cfg: VerifiedCleanupCfg,
    exits: Vec<EmittedExitPath>,
}

fn invalid(pos: Pos, message: &'static str) -> Diagnostic {
    Diagnostic::new(pos, "E_CLEANUP", message)
}

fn ident(value: &str, expected: &str) -> bool { value == expected }

fn lower_exact_body(body: &[Tok], parameter: Option<&str>) -> Option<ReceiverKind> {
    use Tok::*;
    // Closed whitelist: arbitrary expressions, unknown calls, early exits,
    // nesting and additional statements must not inherit an emitted proof.
    match body {
        [LBrace, Ident(r), Ident(s), Dot, Ident(h), LParen,
         Ident(c), Minus, Ident(mode), Pipe, Ident(b), Pipe, Arrow,
         Ident(read_b), Dot, Ident(field), RParen, Semi, RBrace]
            if ident(r,"return") && ident(s,"self") && ident(h,"handle_state")
                && ident(c,"critical") && ident(mode,"read")
                && b == read_b && ident(field,"count") =>
            Some(ReceiverKind::Read),
        [LBrace, Ident(r), Ident(s), Dot, Ident(h), LParen,
         Ident(c), Minus, Ident(mode), Pipe, Ident(b), Pipe, Arrow,
         Ident(read_b), Dot, Ident(field), Plus, Int(Some(value)),
         RParen, Semi, RBrace]
            if ident(r,"return") && ident(s,"self") && ident(h,"handle_state")
                && ident(c,"critical") && ident(mode,"read")
                && b == read_b && ident(field,"count") =>
            i64::try_from(*value).ok().map(ReceiverKind::ReadChecked),
        [LBrace, Ident(s), Dot, Ident(h), LParen,
         Ident(c), Minus, Ident(mode), Pipe, Ident(b), Pipe, Arrow,
         LBrace, Ident(write_b), Dot, Ident(field), Assign,
         Ident(read_b), Dot, Ident(read_field), Plus, Ident(arg),
         Semi, RBrace, RParen, Semi, RBrace]
            if ident(s,"self") && ident(h,"handle_state")
                && ident(c,"critical") && ident(mode,"write")
                && b == write_b && b == read_b
                && ident(field,"count") && ident(read_field,"count")
                && parameter == Some(arg.as_str()) =>
            Some(ReceiverKind::AddArgument),
        _ => None,
    }
}

fn mode(kind: ReceiverKind) -> StateLeaseMode {
    match kind {
        ReceiverKind::AddArgument => StateLeaseMode::ExclusiveWrite,
        _ => StateLeaseMode::SharedRead,
    }
}
fn can_panic(kind: ReceiverKind) -> bool {
    !matches!(kind, ReceiverKind::Read)
}

fn expected_instructions(kind: ReceiverKind, exit: CleanupExit)
    -> Vec<ReceiverInstruction> {
    use ReceiverInstruction::*;
    let mut out = vec![Acquire { lease: 0, mode: mode(kind) }];
    if exit != CleanupExit::CooperativeCancel {
        out.push(Load { lease: 0 });
        match kind {
            ReceiverKind::Read => {}
            ReceiverKind::ReadChecked(value) =>
                out.push(CheckedAddLiteral { lease: 0, value }),
            ReceiverKind::AddArgument => {
                out.push(CheckedAddArgument { lease: 0 });
                if exit == CleanupExit::Normal {
                    out.push(Store { lease: 0 });
                }
            }
        }
    }
    out.push(Release { lease: 0 });
    out.push(Exit(exit));
    out
}

impl ExperimentalReceiver {
    pub fn exits(&self) -> &[EmittedExitPath] { &self.exits }

    /// Validate the emitted instruction streams, not just a frontend boolean.
    /// No branch/suspension or arbitrary callback instruction is in this IR.
    pub fn verify_instructions(&self) -> Result<(), Diagnostic> {
        let expected = if can_panic(self.kind) { 3 } else { 2 };
        if self.cfg.lease_count() != 1 || self.exits.len() != expected
            || self.cfg.paths().len() != expected {
            return Err(invalid(Pos::default(), "incorrect emitted receiver exit coverage"));
        }
        let mut seen = std::collections::BTreeSet::new();
        for path in &self.exits {
            if path.exit == CleanupExit::CheckedPanic && !can_panic(self.kind)
                || !seen.insert(path.exit as u8)
                || path.instructions != expected_instructions(self.kind, path.exit)
                || !self.cfg.paths().iter().any(|checked| {
                    checked.lease == 0 && checked.exit == path.exit && checked.pos == path.pos
                }) {
                return Err(invalid(path.pos,
                    "receiver instructions do not prove exact acquire/release/exit sequence"));
            }
        }
        for exit in [CleanupExit::Normal, CleanupExit::CooperativeCancel] {
            if !seen.contains(&(exit as u8)) {
                return Err(invalid(Pos::default(), "missing receiver cleanup exit"));
            }
        }
        if can_panic(self.kind) && !seen.contains(&(CleanupExit::CheckedPanic as u8)) {
            return Err(invalid(Pos::default(), "missing receiver panic cleanup exit"));
        }
        Ok(())
    }

    /// Only the experimental native Rust lock witness can run this method.
    /// It does not execute emitted machine code or certify a production ABI.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn run_native(&self,
        state: &crate::target_cleanup_cfg::native_experiment::LeaseCell,
        delta: i64,
    ) -> Result<std::thread::Result<i64>, &'static str> {
        self.verify_instructions().map_err(|_| "invalid receiver instruction CFG")?;
        match self.kind {
            ReceiverKind::Read => state.read_snapshot(&self.cfg, 0).map(Ok),
            ReceiverKind::AddArgument => state.checked_add(&self.cfg, 0, delta),
            ReceiverKind::ReadChecked(increment) => {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    state.with_shared(&self.cfg, 0, |value| {
                        value.checked_add(increment)
                            .expect("checked Atto receiver read overflow")
                    })
                }));
                match result {
                    Ok(Ok(value)) => Ok(Ok(value)),
                    Ok(Err(reason)) => Err(reason),
                    Err(panic) => Ok(Err(panic)),
                }
            }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn cancel_native(&self,
        state: &crate::target_cleanup_cfg::native_experiment::LeaseCell,
    ) -> Result<(), &'static str> {
        self.verify_instructions().map_err(|_| "invalid receiver instruction CFG")?;
        // The Rust witness currently exposes a cancellation injection for
        // exclusive writes only. Shared-read cancellation remains unproved.
        match self.kind {
            ReceiverKind::AddArgument => state.cancel_while_exclusive(&self.cfg, 0),
            _ => Err("native shared-read cancellation not implemented"),
        }
    }
}

/// Parse, type-check and lower ONE concrete actor receiver from actual Atto
/// tokens. All other forms are rejected; the main compiler stays fail-closed.
pub fn emit_experimental_receiver(source: &str)
    -> Result<ExperimentalReceiver, Diagnostic> {
    let tokens = lex(source)?;
    let headers = parse_target_declarations(&tokens)?;
    let typed = lower_target_signatures(&headers)?;
    if headers.receiver_bodies.len() != 1 || typed.receivers.len() != 1
        || typed.receiver_cleanup.len() != 1 {
        return Err(invalid(Pos::default(), "experimental receiver requires one complete body"));
    }
    let receiver = &typed.receivers[0];
    let actor = &typed.actors[receiver.actor];
    if !actor.has_definition || typed.state_uses.is_empty()
        || typed.state_uses.iter().any(|use_| {
            use_.receiver != 0 || use_.field != "count"
                || use_.field_ty != Some(TargetType::Int)
        }) {
        return Err(invalid(receiver.pos,
            "experimental receiver requires concrete actor with typed int count field"));
    }
    let param = match receiver.params.as_slice() {
        [] => None,
        [p] if p.ty == TargetType::Int => Some(p.name.as_str()),
        _ => return Err(invalid(receiver.pos,
            "experimental receiver parameters not representable")),
    };
    let kinds: Vec<_> = headers.receiver_bodies[0].tokens.iter()
        .map(|token| token.tok.clone()).collect();
    let kind = lower_exact_body(&kinds, param)
        .ok_or_else(|| invalid(receiver.pos, "receiver body is outside exact instruction subset"))?;
    if (kind == ReceiverKind::AddArgument) != receiver.mutable
        || (kind == ReceiverKind::AddArgument && param.is_none())
        || (kind != ReceiverKind::AddArgument && param.is_some())
        || receiver.returns != if kind == ReceiverKind::AddArgument {
            TargetType::Void
        } else { TargetType::Int } {
        return Err(invalid(receiver.pos, "receiver signature does not match instruction body"));
    }
    let plan = &typed.receiver_cleanup[0];
    if plan.leases.len() != 1 || plan.leases[0].mode != mode(kind)
        || plan.leases[0].checked_arithmetic != can_panic(kind) {
        return Err(invalid(receiver.pos, "typed lease plan disagrees with emitted receiver"));
    }
    let cfg = lower_verified_cleanup_cfg(plan)?;
    let exits = cfg.paths().iter().map(|path| EmittedExitPath {
        exit: path.exit,
        pos: path.pos,
        instructions: expected_instructions(kind, path.exit),
    }).collect();
    let generated = ExperimentalReceiver { kind, cfg, exits };
    generated.verify_instructions()?;
    Ok(generated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target_cleanup_cfg::native_experiment::LeaseCell;

    const READ: &str = "declare actor Counter; define actor Counter as let int count = 0; end
        mtd (Counter self) current(): int {
            return self.handle_state(critical-read |s| -> s.count);
        }";
    const READ_CHECKED: &str = "declare actor Counter; define actor Counter as let int count = 0; end
        mtd (Counter self) current(): int {
            return self.handle_state(critical-read |s| -> s.count + 1);
        }";
    const ADD: &str = "declare actor Counter; define actor Counter as let int count = 0; end
        mtd (mut self: Counter) increment(int delta): void {
            self.handle_state(critical-write |s| -> { s.count = s.count + delta; });
        }";

    #[test]
    fn generated_source_receiver_paths_verify_without_opening_production_gate() {
        for (source, count) in [(READ,2),(READ_CHECKED,3),(ADD,3)] {
            let generated = emit_experimental_receiver(source).unwrap();
            assert_eq!(generated.exits().len(), count);
            generated.verify_instructions().unwrap();
            assert_eq!(crate::compile(source).unwrap_err().code, "E_UNSUPPORTED");
        }
    }

    #[test]
    fn generated_exit_verifier_rejects_deleted_duplicated_and_reordered_releases() {
        let good = emit_experimental_receiver(ADD).unwrap();
        let mut broken = good.clone();
        broken.exits[0].instructions.retain(|instruction| {
            !matches!(instruction, ReceiverInstruction::Release { .. })
        });
        assert_eq!(broken.verify_instructions().unwrap_err().code, "E_CLEANUP");
        let mut broken = good.clone();
        broken.exits[0].instructions.insert(3, ReceiverInstruction::Release { lease: 0 });
        assert_eq!(broken.verify_instructions().unwrap_err().code, "E_CLEANUP");
        let mut broken = good.clone();
        broken.exits.swap(0,1);
        broken.exits[0].pos = Pos::default();
        assert_eq!(broken.verify_instructions().unwrap_err().code, "E_CLEANUP");
        let mut broken = good;
        broken.exits.retain(|e| e.exit != CleanupExit::CheckedPanic);
        assert_eq!(broken.verify_instructions().unwrap_err().code, "E_CLEANUP");
    }

    #[test]
    fn generated_receiver_native_witness_releases_on_normal_unwind_and_cancel() {
        let read = emit_experimental_receiver(READ).unwrap();
        let checked_read = emit_experimental_receiver(READ_CHECKED).unwrap();
        let add = emit_experimental_receiver(ADD).unwrap();
        let cell = LeaseCell::new(i64::MAX - 1);
        assert_eq!(read.run_native(&cell, 0).unwrap().unwrap(), i64::MAX-1);
        assert_eq!(checked_read.run_native(&cell, 0).unwrap().unwrap(), i64::MAX);
        assert_eq!(add.run_native(&cell, 1).unwrap().unwrap(), i64::MAX);
        assert!(add.run_native(&cell, 1).unwrap().is_err());
        assert!(checked_read.run_native(&cell, 0).unwrap().is_err());
        assert_eq!(add.cancel_native(&cell), Err("cancelled"));
        assert_eq!(cell.release_count(), 6);
        assert!(cell.try_write_available());
    }

    #[test]
    fn experimental_emitter_fails_closed_on_unrepresentable_source() {
        let rewritten = ADD.replace("s.count + delta", "delta + s.count");
        assert_eq!(emit_experimental_receiver(&rewritten).unwrap_err().code, "E_CLEANUP");
        let extra = ADD.replace("});", "}); self.handle_state(critical-read |s| -> s.count);");
        assert_eq!(emit_experimental_receiver(&extra).unwrap_err().code, "E_CLEANUP");
    }
}
