//! Validated scalar MIR. This is intentionally an Oreslang IR, not Cranelift IR.
use crate::frontend::{Expr, Program};
use std::collections::{HashMap, HashSet};
use std::fmt;

#[derive(Clone, Debug)]
pub struct Module { pub functions: Vec<Function> }
#[derive(Clone, Debug)]
pub struct Function {
    pub symbol: String,
    pub exported: bool,
    pub params: Vec<String>,
    pub locals: Vec<(String, Expr)>,
    pub return_value: Expr,
}
#[derive(Debug, Clone)]
pub struct SemanticError {
    pub class: &'static str,
    pub message: String,
}
impl fmt::Display for SemanticError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.class, self.message)
    }
}
impl std::error::Error for SemanticError {}
fn failure(class: &'static str, message: impl Into<String>) -> SemanticError {
    SemanticError { class, message: message.into() }
}
/// Preflight the shape of checked or externally supplied MIR without recursion.
pub(crate) fn validate_shape(expr: &Expr) -> Result<(), String> {
    let mut pending = vec![(expr, 1usize)];
    let mut visited = 0usize;
    while let Some((node, depth)) = pending.pop() {
        visited += 1;
        if visited > 4096 || depth > 128 {
            return Err("expression exceeds node/depth budget".into());
        }
        match node {
            Expr::Integer(_) | Expr::Name(_) => {}
            Expr::Negate(v) => pending.push((v, depth + 1)),
            Expr::Add(a, b) | Expr::Subtract(a, b) | Expr::Multiply(a, b) => {
                pending.push((a, depth + 1));
                pending.push((b, depth + 1));
            }
            Expr::Call { args, .. } => {
                pending.extend(args.iter().map(|arg| (arg, depth + 1)));
            }
        }
    }
    Ok(())
}
fn validate_expr(
    expr: &Expr, variables: &HashSet<String>, signatures: &HashMap<String, usize>,
) -> Result<(), SemanticError> {
    match expr {
        Expr::Integer(_) => Ok(()),
        Expr::Name(name) => {
            if variables.contains(name) { Ok(()) }
            else { Err(failure("unknown_name", format!("undeclared scalar '{name}'"))) }
        }
        Expr::Call { name, args } => {
            let Some(expected) = signatures.get(name) else {
                return Err(failure("unknown_name", format!("unknown function '{name}'")));
            };
            if args.len() != *expected {
                return Err(failure("call_arity", format!("{name}: expected {expected} parameters, received {}", args.len())));
            }
            for arg in args { validate_expr(arg, variables, signatures)?; }
            Ok(())
        }
        Expr::Add(a, b) | Expr::Subtract(a, b) | Expr::Multiply(a, b) => {
            validate_expr(a, variables, signatures)?;
            validate_expr(b, variables, signatures)
        }
        Expr::Negate(expr) => validate_expr(expr, variables, signatures),
    }
}
pub fn lower(program: Program) -> Result<Module, SemanticError> {
    if program.functions.is_empty() || program.functions.len() > 1024 {
        return Err(failure("resource_budget", "function count outside supported bound"));
    }
    let mut signatures = HashMap::new();
    for function in &program.functions {
        if function.params.len() > 256 || function.locals.len() > 1024 {
            return Err(failure("resource_budget", "function parameters/locals exceed budget"));
        }
        validate_shape(&function.expression)
            .map_err(|e| failure("resource_budget", e))?;
        for local in &function.locals {
            validate_shape(&local.value).map_err(|e| failure("resource_budget", e))?;
        }
        if signatures.insert(function.name.clone(), function.params.len()).is_some() {
            return Err(failure("duplicate_declaration", format!("duplicate function '{}'", function.name)));
        }
    }
    let mut functions = Vec::with_capacity(program.functions.len());
    for function in program.functions {
        let mut vars = HashSet::new();
        for param in &function.params {
            if !vars.insert(param.clone()) {
                return Err(failure("duplicate_declaration", format!("duplicate parameter '{param}'")));
            }
        }
        let mut locals = Vec::new();
        for local in function.locals {
            validate_expr(&local.value, &vars, &signatures)?;
            if !vars.insert(local.name.clone()) {
                return Err(failure("duplicate_declaration", format!("duplicate local '{}'", local.name)));
            }
            locals.push((local.name, local.value));
        }
        validate_expr(&function.expression, &vars, &signatures)?;
        functions.push(Function {
            symbol: function.name,
            exported: function.exported,
            params: function.params,
            locals,
            return_value: function.expression,
        });
    }
    Ok(Module { functions })
}
#[cfg(test)]
mod tests {
    use super::*;
    fn check(source: &str) -> Result<Module, SemanticError> {
        lower(crate::frontend::parse(source).unwrap())
    }
    #[test]
    fn rejects_left_deep_arithmetic_before_recursive_lowering() {
        let src = format!("fnc f(): int {{ return 1{}; }}", "+1".repeat(200));
        assert_eq!(check(&src).unwrap_err().class, "resource_budget");
    }
    #[test]
    fn duplicate_function_rejected() {
        assert_eq!(check("fnc f(): int { return 1; } fnc f(): int { return 2; }").unwrap_err().class, "duplicate_declaration");
    }
    #[test]
    fn forward_call_admitted() {
        check("pub fnc main(): int { return times(7,6); } fnc times(int a,int b): int { return a*b; }").unwrap();
    }
    #[test]
    fn unknown_name_rejected() {
        assert_eq!(check("pub fnc main(): int { return neverDeclared; }").unwrap_err().class, "unknown_name");
    }
    #[test]
    fn call_arity_rejected() {
        assert_eq!(check("fnc add(int a,int b): int { return a+b; } pub fnc main(): int { return add(1); }").unwrap_err().class, "call_arity");
    }
    #[test]
    fn shadowed_local_rejected() {
        assert_eq!(check("fnc f(int a): int { const a: int = 3; return a; }").unwrap_err().class, "duplicate_declaration");
    }
    #[test]
    fn local_binding_order_enforced() {
        assert_eq!(check("fnc f(): int { const a: int = b; return a; }").unwrap_err().class, "unknown_name");
    }
}
