//! Owned Oreslang MIR shell, intentionally distinct from Cranelift IR.
use crate::frontend::{Expr, Program};
use std::collections::HashSet;
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
    fn duplicate_symbol_rejected() {
        let p = crate::frontend::parse(
            "static fnc answer() as return 1; end static fnc answer() as return 2; end"
        ).unwrap();
        assert!(lower(p).is_err());
    }
}
