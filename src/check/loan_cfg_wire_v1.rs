//! Portable closed-profile facts for the independent atto-borrow-checker.rs
//! SSA oracle. Versioned plain-text contract, not signed compiler evidence.
//!
//! The producer first validates its actual checked typed IR through the
//! separate branch CFG oracle; it never serializes caller-authored loan ends
//! as inferred liveness. Do NOT use the external verdict for codegen admission.
use crate::check::{
    loan_cfg_oracle_v1::{self, Kind},
    TProgram,
};
use crate::diag::{Diagnostic, Pos};

pub const CONTRACT: &str = "ATTO_LOAN_CFG_V1";
const MAX_MANIFEST: usize = 1_048_576;

fn unsupported(message: &'static str) -> Diagnostic {
    Diagnostic::new(Pos::default(), "E_EXTERNAL_LOAN_CFG_UNSUPPORTED", message)
}
fn list(items: impl IntoIterator<Item = u32>) -> String {
    let mut out = String::new();
    for x in items {
        if !out.is_empty() {
            out.push(',');
        }
        out.push_str(&x.to_string());
    }
    if out.is_empty() {
        out.push('-');
    }
    out
}
/// Stable manifest of every CFG point, edge, loan issue/end, write,
/// typed borrowed load, direct owner read, and conservative trap flag
/// admitted by the compiler's closed scalar CFG audit.
///
/// Source provenance and emitted op coverage must be independently attested
/// before *either* diagnostic result could become a safety gate.
pub fn export_checked_program(program: &TProgram) -> Result<String, Diagnostic> {
    let report = loan_cfg_oracle_v1::audit_checked_program(program)?;
    let mut out = format!("{CONTRACT}\n");
    for function in report.functions {
        if function.name.is_empty()
            || !function.name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            || function.points.len() > 1024
        {
            return Err(unsupported("function name or CFG exceeds stable external protocol"));
        }
        out.push_str(&format!("F|{}|{}|{}\n", function.name, function.entry + 1, function.points.len()));
        for point in &function.points {
            let (tag, loan, owner, flag, via) = match point.kind {
                Kind::Issue {
                    loan_id,
                    owner_slot,
                    exclusive,
                } => ("I", loan_id, owner_slot, u32::from(exclusive), 0),
                Kind::Marker {
                    loan_id,
                    owner_slot,
                    write,
                } => ("M", loan_id, owner_slot, u32::from(write), 0),
                Kind::End {
                    loan_id,
                    owner_slot,
                    lexical,
                } => ("E", loan_id, owner_slot, u32::from(lexical), 0),
                Kind::Set { owner_slot, via } => ("S", 0, owner_slot, 0, via.unwrap_or(0)),
                Kind::Eval => ("V", 0, 0, 0, 0),
                Kind::Condition => ("C", 0, 0, 0, 0),
                Kind::Return => ("R", 0, 0, 0, 0),
            };
            let borrowed = if point.borrowed_reads.is_empty() {
                "-".to_string()
            } else {
                point
                    .borrowed_reads
                    .iter()
                    .map(|(loan, slot)| format!("{loan}:{slot}"))
                    .collect::<Vec<_>>()
                    .join(",")
            };
            let successors = list(point.successors.iter().map(|v| v + 1));
            let direct = list(point.direct_reads);
            out.push_str(&format!(
                "P|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}\n",
                point.id + 1,
                tag,
                loan,
                owner,
                flag,
                via,
                successors,
                borrowed,
                direct,
                u8::from(point.may_trap),
                u8::from(point.conditional),
            ));
            if out.len() > MAX_MANIFEST {
                return Err(unsupported("external loan census exceeds wire byte budget"));
            }
        }
    }
    out.push_str("END\n");
    if out.len() > MAX_MANIFEST {
        return Err(unsupported("external loan census exceeds wire byte budget"));
    }
    Ok(out)
}
