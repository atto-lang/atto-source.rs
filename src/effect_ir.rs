//! Conservative v1 effect-IR export from the *validated scalar MIR*.
//!
//! This proves only that the currently admitted Rust scalar subset contains
//! literals, integer arithmetic and no guest allocations, calls, I/O, FFI,
//! suspension, actor transfers or loops. It is NOT a trusted WCET certificate.
//! Future MIR variants MUST be rejected until exhaustively classified.
use crate::{frontend, mir::{self, Module}};
use std::collections::HashSet;

fn valid_symbol(symbol: &str) -> bool {
    let mut chars = symbol.bytes();
    matches!(chars.next(), Some(b'a'..=b'z' | b'A'..=b'Z' | b'_'))
        && symbol.len() <= 256
        && chars.all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

/// Encode schema_version=1 accepted by oreslang-static-analysis.rs.
///
/// Every function is enumerated from checked MIR (not from string-search of
/// source code). Empty operations mean *zero modeled guest effects* in this
/// explicitly closed-world scalar subset. Compile-time Cranelift allocations
/// are not guest allocations. This does NOT establish instruction/stack/WCET
/// bounds or authenticate the producer of the JSON.
pub fn emit_effect_ir(module: &Module) -> Result<String, String> {
    if module.functions.is_empty() || module.functions.len() > 1024 {
        return Err("effect IR requires a bounded nonempty module".into());
    }
    let mut names = HashSet::new();
    for function in &module.functions {
        if !valid_symbol(&function.symbol) || function.symbol == "main" {
            return Err("invalid or unsupported effect-IR function symbol".into());
        }
        if !names.insert(function.symbol.as_str()) {
            return Err("duplicate effect-IR function symbol".into());
        }
        // Exhaustively traverses the closed MIR Expr enum. Adding a new
        // variant forces the compiler to revisit MIR admission and exporter.
        mir::validate_expr(&function.return_value)?;
    }
    let mut symbols: Vec<_> = names.into_iter().collect();
    symbols.sort_unstable();
    let mut out = String::from("{\"schema_version\":1,\"functions\":[");
    for (i, symbol) in symbols.iter().enumerate() {
        if i != 0 { out.push(','); }
        // Safe without general-purpose JSON escaping: the identifier grammar
        // is ASCII letters/digits/underscore, checked above.
        out.push_str("{\"name\":\"");
        out.push_str(symbol);
        out.push_str("\",\"operations\":[]}");
    }
    out.push_str("]}");
    Ok(out)
}

pub fn source_to_effect_ir(source: &str) -> Result<String, String> {
    let ast = frontend::parse(source).map_err(|e| e.to_string())?;
    emit_effect_ir(&mir::lower(ast)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::Expr;
    #[test]
    fn emits_all_checked_source_functions_sorted() {
        let src = "static fnc zebra() as return 3 * 7; end\n\
                   static fnc alpha() as return 40 + 2; end";
        let json = source_to_effect_ir(src).unwrap();
        assert_eq!(json, "{\"schema_version\":1,\"functions\":[{\"name\":\"alpha\",\"operations\":[]},{\"name\":\"zebra\",\"operations\":[]}]}");
    }
    #[test]
    fn no_analysis_for_unlowered_actor_or_io() {
        assert!(source_to_effect_ir("define actor Example as end").is_err());
        assert!(source_to_effect_ir("static fnc a() as return readch(ch); end").is_err());
        assert!(source_to_effect_ir("static fnc a() as return 1; end static fnc a() as return 2; end").is_err());
    }
    #[test]
    fn direct_module_api_cannot_smuggle_json_or_duplicate_symbols() {
        let function = mir::Function { symbol: "bad\"injection".into(), return_value: Expr::Integer(1) };
        assert!(emit_effect_ir(&mir::Module { functions: vec![function] }).is_err());
        let function = mir::Function { symbol: "safe".into(), return_value: Expr::Integer(1) };
        assert!(emit_effect_ir(&mir::Module { functions: vec![function.clone(), function] }).is_err());
    }
    #[test]
    fn deterministic_under_function_reordering() {
        let f = |s: &str| mir::Function { symbol: s.into(), return_value: Expr::Integer(0) };
        let a = emit_effect_ir(&mir::Module { functions: vec![f("z"), f("a")] }).unwrap();
        let b = emit_effect_ir(&mir::Module { functions: vec![f("a"), f("z")] }).unwrap();
        assert_eq!(a, b);
    }
}
