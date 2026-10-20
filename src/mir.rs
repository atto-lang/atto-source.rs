//! Validated scalar MIR. This is intentionally an Oreslang IR, not Cranelift IR.
use crate::frontend::{Expr, Program};
use std::collections::{HashMap, HashSet};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutScope {
    Core,
    Stdlib,
    Runtime,
    Userland,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhysicalLayout {
    Aos,
    Soa,
    Aosoa,
    SparseSet,
    Archetype,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayoutRegion {
    pub name: String,
    pub scope: LayoutScope,
    pub layout: PhysicalLayout,
    pub rows: Option<u64>,
    pub field_bytes: Vec<u32>,
    pub accessed_fields: Vec<u32>,
    pub dense_iteration: bool,
    pub sequential_rows: bool,
    pub hot: bool,
    pub relocation_verified: bool,
    pub stable_address_required: bool,
    pub whole_record_access: bool,
    pub iteration_independent: bool,
    pub sparse_membership: bool,
    pub independent_component_passes: bool,
    pub concurrent_mutable_sharing: bool,
}

#[derive(Clone, Debug)]
pub struct Module {
    pub functions: Vec<Function>,
    /// True only when every layout-bearing MIR region is represented below.
    /// The currently admitted scalar subset has no such regions, so lowering
    /// can prove this with an empty list.
    pub layout_evidence_complete: bool,
    pub layout_regions: Vec<LayoutRegion>,
}
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
    let mut signatures = HashMap::new();
    for function in &program.functions {
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
    Ok(Module {
        functions,
        layout_evidence_complete: true,
        layout_regions: Vec::new(),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    fn check(source: &str) -> Result<Module, SemanticError> {
        lower(crate::frontend::parse(source).unwrap())
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
