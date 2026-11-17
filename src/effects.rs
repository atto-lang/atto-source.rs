//! Backend-neutral effect-IR v1 adapter for oreslang-static-analysis.rs.
//! A complete semantic/MIR pass must cover every operation before export.
use crate::{
    frontend::Expr,
    mir::{self, Module},
};
use serde_json::{json, Value};

fn analyze_expr(expr: &Expr, out: &mut Vec<Value>) {
    match expr {
        Expr::Integer(_) | Expr::Name(_) => {
            out.push(json!({"kind":"compute", "units": 1}));
        }
        Expr::Negate(v) => {
            analyze_expr(v, out);
            out.push(json!({"kind":"compute", "units": 1}));
        }
        Expr::Add(a, b) | Expr::Subtract(a, b) | Expr::Multiply(a, b) => {
            analyze_expr(a, out);
            analyze_expr(b, out);
            out.push(json!({"kind":"compute", "units": 1}));
        }
        Expr::Call { name, args } => {
            for arg in args {
                analyze_expr(arg, out);
            }
            out.push(json!({"kind":"call", "target":name}));
        }
    }
}

/// This is only complete for the admitted scalar subset: no allocation, FFI,
/// unbounded waits, dynamic calls, or actor mutation operations exist in it.
/// Abstract compute units are NOT CPU instructions, WCET or timing guarantees.
pub fn export_effect_ir(module: &Module) -> Result<Value, String> {
    mir::validate_module(module)?;
    let functions: Vec<_> = module
        .functions
        .iter()
        .map(|func| {
            let mut operations = Vec::new();
            for (_, expression) in &func.locals {
                analyze_expr(expression, &mut operations);
            }
            analyze_expr(&func.return_value, &mut operations);
            json!({"name": func.symbol, "operations": operations})
        })
        .collect();
    Ok(json!({"schema_version": 1, "functions": functions}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_external_module_cannot_forge_a_pure_call_graph() {
        let invalid = mir::Module {
            functions: vec![mir::Function {
                symbol: "entry".into(),
                exported: true,
                params: vec![],
                locals: vec![],
                return_value: Expr::Call {
                    name: "unknown".into(),
                    args: vec![],
                },
            }],
        };
        assert!(export_effect_ir(&invalid).is_err());
    }
    #[test]
    fn tracks_forward_calls() {
        let ast = crate::frontend::parse(
            "pub fnc main(): int { return add(40,2); }\nfnc add(int a,int b): int { return a+b; }",
        )
        .unwrap();
        let mir = crate::mir::lower(ast).unwrap();
        let ir = export_effect_ir(&mir).unwrap();
        assert_eq!(ir["functions"][0]["operations"][2]["kind"], "call");
        assert_eq!(ir["functions"][0]["operations"][2]["target"], "add");
    }
}
