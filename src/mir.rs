//! Owned Oreslang MIR shell, intentionally distinct from Cranelift IR.
use crate::frontend::{Expr, Program};
use std::collections::HashSet;
const MAX_MIR_EXPR_DEPTH: usize = 128;
fn validate_expr(root: &Expr) -> Result<(), String> {
    let mut pending = vec![(root, 1usize)];
    while let Some((expr, depth)) = pending.pop() {
        if depth > MAX_MIR_EXPR_DEPTH {
            return Err("expression exceeds MIR/codegen stack budget".into());
        }
        match expr {
            Expr::Integer(_) => {}
            Expr::Add(a, b) | Expr::Subtract(a, b) | Expr::Multiply(a, b) => {
                pending.push((a, depth + 1));
                pending.push((b, depth + 1));
            }
        }
    }
    Ok(())
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Module { pub functions: Vec<Function> }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Function { pub symbol: String, pub return_value: Expr }

pub fn lower(program: Program) -> Result<Module, String> {
    let mut names = HashSet::new();
    let mut functions = Vec::new();
    for function in program.functions {
        if !names.insert(function.name.clone()) {
            return Err(format!("duplicate function: {}", function.name));
        }
        validate_expr(&function.expression)?;
        if function.name == "main" {
            return Err("native main ABI not yet defined: export a named function".into());
        }
        functions.push(Function { symbol: function.name, return_value: function.expression });
    }
    Ok(Module { functions })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_deep_left_associative_ast_before_codegen() {
        let source = format!("static fnc f() as return 1{}; end", "+1".repeat(200));
        let parsed = crate::frontend::parse(&source).unwrap();
        assert!(lower(parsed).is_err());
    }
    #[test]
    fn duplicate_symbol_rejected() {
        let p = crate::frontend::parse(
            "static fnc answer() as return 1; end static fnc answer() as return 2; end"
        ).unwrap();
        assert!(lower(p).is_err());
    }
}
