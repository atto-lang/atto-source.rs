//! Opt-in executable Wasm cleanup *witness* for the checked actor state ABI.
//! This deliberately does NOT compile the actor callback or enable mtd codegen.
use crate::diag::{Diagnostic,Pos};
use crate::target_cleanup::StateLeaseMode;
use crate::target_cleanup_cfg::lower_verified_cleanup_cfg;

/// Emit a separate witness module proving that the backend can encode and
/// inspect concrete release instructions on normal, checked-overflow and
/// cooperative-cancellation paths. The method's source callback is only used
/// for validated cleanup *obligations*: the executable operation is a fixed
/// synthetic i64 update, not the AttoLang state-body semantics.
pub fn compile_cleanup_wasm_witness(source:&str)->Result<Vec<u8>,Diagnostic>{
    if source.len()>crate::SOURCE_CAP {
        return Err(Diagnostic::new(Pos{line:1,col:1},"E_SOURCE_LIMIT",
            "source exceeds cleanup witness input cap"));
    }
    let toks=crate::lexer::lex(source)?;
    let headers=crate::target_syntax::parse_target_declarations(&toks)?;
    let typed=crate::target_typed::lower_target_signatures(&headers)?;
    let actor_ast=crate::target_typed::build_actor_program(&headers,&typed);
    crate::check::check_target_actor_interface(&actor_ast)?;
    if typed.receiver_cleanup.len()!=1 {
        return Err(Diagnostic::new(Pos::default(),"E_UNSUPPORTED",
            "cleanup Wasm witness admits exactly one typed actor receiver"));
    }
    let plan=&typed.receiver_cleanup[0];
    if plan.leases.len()!=1 || plan.leases[0].mode!=StateLeaseMode::ExclusiveWrite
        || !plan.leases[0].checked_arithmetic {
        return Err(Diagnostic::new(Pos::default(),"E_UNSUPPORTED",
            "cleanup Wasm witness requires one checked exclusive state lease"));
    }
    // Verifier rejects missing release obligations, forged proof flags,
    // out-of-order regions and unmatched exits before any Wasm is emitted.
    let verified=lower_verified_cleanup_cfg(plan)?;
    let bytes=crate::wasm::emit_cleanup_wasm_witness(&verified)?;
    if bytes.len()>crate::wasm::MAX_OUTPUT {
        return Err(Diagnostic::new(Pos::default(),"E_OUTPUT_LIMIT",
            "cleanup witness Wasm output exceeds limit"));
    }
    Ok(bytes)
}

/// Attests the *actual Wasm code section* against the tightly bounded
/// three-path instruction contract. No frontend proof bit is trusted.
pub fn audit_cleanup_wasm_witness(bytes:&[u8])->Result<(),Diagnostic>{
    crate::wasm::audit_cleanup_wasm_witness(bytes)
}
