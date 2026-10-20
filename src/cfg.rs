//! CFG-v2 exporter for the compiler's checked scalar MIR subset.
//! Only a single explicit return block is currently generated. This is
//! compatible with the CFG verifier, but NOT a full control-flow lowering.
use crate::{frontend::Expr, mir::Module};
use serde_json::{json, Value};

fn append(expr: &Expr, events: &mut Vec<Value>) {
    match expr {
        Expr::Integer(_) => events.push(json!({"kind": "compute", "units": 1})),
        Expr::Name(name) => events.push(json!({"kind": "read", "place": name})),
        Expr::Negate(a) => {
            append(a, events);
            events.push(json!({"kind": "compute", "units": 1}));
        }
        Expr::Add(a, b) | Expr::Subtract(a, b) | Expr::Multiply(a, b) => {
            append(a, events);
            append(b, events);
            events.push(json!({"kind": "compute", "units": 1}));
        }
        Expr::Call { name, args } => {
            for arg in args {
                append(arg, events);
            }
            // Until interprocedural summaries are verified, this call MUST
            // fail closed in the CFG safety admission engine.
            events.push(json!({"kind": "call", "target": name}));
        }
    }
}

/// The budget is provided by policy, not computed from CPU timing/WCET.
/// Runtime actor/Task/memory effects cannot occur in the admitted scalar slice.
pub fn export_cfg_v2(module: &Module, budget: Option<u64>) -> Value {
    let functions = module
        .functions
        .iter()
        .map(|function| {
            let mut events = Vec::new();
            for name in &function.params {
                events.push(json!({"kind": "define", "place": name}));
            }
            for (name, expr) in &function.locals {
                append(expr, &mut events);
                events.push(json!({"kind": "define", "place": name}));
            }
            append(&function.return_value, &mut events);
            json!({
                "name": function.symbol,
                "entry": "entry",
                "max_uninterrupted_units": budget,
                "allow_error_propagation": false,
                "blocks": [
                    {"id": "entry", "events": events, "terminator": {"kind": "return"}}
                ]
            })
        })
        .collect::<Vec<_>>();
    json!({
        "schema_version": 2,
        "effects_complete": true,
        "functions": functions
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    fn export(source: &str, budget: Option<u64>) -> Value {
        let ast = crate::frontend::parse(source).unwrap();
        let mir = crate::mir::lower(ast).unwrap();
        export_cfg_v2(&mir, budget)
    }
    #[test]
    fn scalar_expression_has_one_cfg_return_block() {
        let v = export("pub fnc answer(): int { return 40 + 2; }", Some(10));
        assert_eq!(v["schema_version"], 2);
        assert_eq!(v["functions"][0]["entry"], "entry");
        assert_eq!(
            v["functions"][0]["blocks"][0]["events"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        assert_eq!(
            v["functions"][0]["blocks"][0]["terminator"]["kind"],
            "return"
        );
    }
    #[test]
    fn calls_remain_explicit_and_unverified() {
        let v = export(
            "pub fnc main(): int { return add(40,2); } fnc add(int a,int b): int { return a+b; }",
            Some(10),
        );
        assert!(v["functions"][0]["blocks"][0]["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["kind"] == "call" && x["target"] == "add"));
    }
    #[test]
    fn parameter_and_local_places_are_declared_before_read() {
        let v = export(
            "pub fnc use_number(int x): int { const y: int = x; return y; }",
            Some(10),
        );
        let e = v["functions"][0]["blocks"][0]["events"].as_array().unwrap();
        assert_eq!(e[0]["kind"], "define");
        assert_eq!(e[1]["kind"], "read");
        assert_eq!(e[2]["kind"], "define");
        assert_eq!(e[3]["kind"], "read");
    }
    #[test]
    fn absent_budget_stays_absent_and_cannot_be_invented() {
        let v = export("pub fnc answer(): int { return 2; }", None);
        assert_eq!(v["functions"][0]["max_uninterrupted_units"], Value::Null);
    }
}
