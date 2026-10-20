//! Conservative v1 effect-IR export from the *validated scalar MIR*.
//!
//! This proves only that the currently admitted Rust scalar subset contains
//! literals, integer arithmetic and no guest allocations, calls, I/O, FFI,
//! suspension, actor transfers or loops. It is NOT a trusted WCET certificate.
//! Future MIR variants MUST be rejected until exhaustively classified.
use crate::{frontend, mir::{self, Module}};
use crate::frontend::Expr;
use std::collections::HashSet;

/// Match the consumer's total effect-operation budget so any emitted IR can
/// actually be analyzed. This is a finite-IR size cap, not a WCET bound.
const MAX_EMITTED_OPERATIONS: usize = 100_000;

/// Count every scalar MIR node without recursion; the MIR validator ensures
/// that codegen and analysis agree on the exact expression vocabulary.
fn scalar_node_count(expr: &Expr) -> Result<usize, String> {
    mir::validate_expr(expr)?;
    let mut count = 0usize;
    let mut pending = vec![expr];
    while let Some(node) = pending.pop() {
        count = count.checked_add(1).ok_or("scalar node count overflow")?;
        match node {
            Expr::Integer(_) => {}
            Expr::Add(left, right)
            | Expr::Subtract(left, right)
            | Expr::Multiply(left, right) => {
                pending.push(right);
                pending.push(left);
            }
        }
    }
    Ok(count)
}


fn valid_symbol(symbol: &str) -> bool {
    let mut chars = symbol.bytes();
    matches!(chars.next(), Some(b'a'..=b'z' | b'A'..=b'Z' | b'_'))
        && symbol.len() <= 256
        && chars.all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

/// Encode schema_version=1 accepted by oreslang-static-analysis.rs.
///
/// Each literal or arithmetic expression contributes exactly one abstract
/// `compute` effect unit, corresponding to its accepted scalar MIR node.
/// This is an upper bound on *modeled nodes*, never a machine cycle/WCET proof.
/// Producer JSON can be forged; provenance must be checked independently.
pub fn emit_effect_ir(module: &Module) -> Result<String, String> {
    if module.functions.is_empty() || module.functions.len() > 1024 {
        return Err("effect IR requires a bounded nonempty module".into());
    }
    let mut names = HashSet::new();
    let mut checked = Vec::with_capacity(module.functions.len());
    let mut total_nodes = 0usize;
    for function in &module.functions {
        if !valid_symbol(&function.symbol) || function.symbol == "main" {
            return Err("invalid or unsupported effect-IR function symbol".into());
        }
        if !names.insert(function.symbol.as_str()) {
            return Err("duplicate effect-IR function symbol".into());
        }
        // Exhaustive Expr matching enforces a compile-time error if the
        // scalar MIR vocabulary grows without corresponding effect admission.
        let nodes = scalar_node_count(&function.return_value)?;
        total_nodes = total_nodes.checked_add(nodes).ok_or("effect IR node count overflow")?;
        if total_nodes > MAX_EMITTED_OPERATIONS {
            return Err("effect IR exceeds analyzer operation budget".into());
        }
        checked.push((function.symbol.as_str(), nodes));
    }
    checked.sort_unstable_by_key(|(symbol, _)| *symbol);
    let mut out = String::from("{\"schema_version\":1,\"functions\":[");
    for (i, (symbol, nodes)) in checked.iter().enumerate() {
        if i != 0 { out.push(','); }
        // Identifiers are restricted to ASCII [A-Za-z_][A-Za-z0-9_]*.
        out.push_str("{\"name\":\"");
        out.push_str(symbol);
        out.push_str("\",\"operations\":[");
        for node in 0..*nodes {
            if node != 0 { out.push(','); }
            out.push_str("{\"kind\":\"compute\",\"units\":1}");
        }
        out.push_str("]}");
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
    fn matches_analyzer_golden_fixture_byte_for_byte() {
        let actual = source_to_effect_ir(include_str!("../examples/answer.ores")).unwrap();
        let expected = include_str!("../examples/scalar-effect-ir-v1.json").trim_end();
        assert_eq!(actual, expected);
        assert_eq!(actual.matches("\"kind\":\"compute\"").count(), 3);
    }

    #[test]
    fn emits_all_checked_source_functions_sorted() {
        let src = "static fnc zebra() as return 3 * 7; end\n\
                   static fnc alpha() as return 40 + 2; end";
        let json = source_to_effect_ir(src).unwrap();
        let alpha = json.find("\"name\":\"alpha\"").unwrap();
        let zebra = json.find("\"name\":\"zebra\"").unwrap();
        assert!(alpha < zebra);
        assert_eq!(json.matches("\"kind\":\"compute\"").count(), 6);
    }

    #[test]
    fn nested_scalar_expression_counts_all_nodes() {
        let src = "static fnc f() as return (1 + 2) * (3 - 4); end";
        let json = source_to_effect_ir(src).unwrap();
        assert_eq!(json.matches("\"kind\":\"compute\"").count(), 7);
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

    #[test]
    fn rejects_export_larger_than_analyzer_operation_budget() {
        // An actual codegen node cannot be silently omitted to fit a budget.
        let make_tree = || {
            let mut leaves = vec![Expr::Integer(1); 128];
            while leaves.len() > 1 {
                leaves = leaves.chunks_exact(2).map(|c|
                    Expr::Add(Box::new(c[0].clone()), Box::new(c[1].clone()))
                ).collect();
            }
            leaves.pop().unwrap()  // 255 nodes
        };
        let tree = make_tree();
        let module = mir::Module {
            functions: (0..400).map(|n| mir::Function {
                symbol: format!("f{n}"), return_value: tree.clone()
            }).collect(),
        };
        assert!(emit_effect_ir(&module).unwrap_err().contains("operation budget"));
    }
}
