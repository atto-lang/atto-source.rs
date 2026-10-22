//! Backend-neutral effect-IR v1 adapter for oreslang-static-analysis.rs.
//! A complete semantic/MIR pass must cover every operation before export.
use crate::{frontend::Expr, mir::Module};
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
pub fn export_effect_ir(module: &Module) -> Value {
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
    json!({"schema_version": 1, "functions": functions})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tracks_forward_calls() {
        let ast = crate::frontend::parse(
            "pub fnc main(): int { return add(40,2); }\nfnc add(int a,int b): int { return a+b; }",
        )
        .unwrap();
        let mir = crate::mir::lower(ast).unwrap();
        let ir = export_effect_ir(&mir);
        assert_eq!(ir["functions"][0]["operations"][2]["kind"], "call");
        assert_eq!(ir["functions"][0]["operations"][2]["target"], "add");
    }
}
