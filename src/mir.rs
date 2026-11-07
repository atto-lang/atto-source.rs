//! Validated scalar MIR. This is intentionally an Oreslang IR, not Cranelift IR.
use crate::{
    frontend::{Expr, Program},
    ref_layout::ManagedRefLayout,
};
use std::collections::HashMap;
use std::fmt;

#[derive(Clone, Debug)]
pub struct Module {
    pub functions: Vec<Function>,
    pub managed_refs: ManagedRefLayout,
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
    SemanticError {
        class,
        message: message.into(),
    }
}
// A value is known only when every executed input to the expression is a
// statically known i64. This is a sound, intentionally incomplete admission
// check, NOT a substitute for checked runtime code generation.
fn validate_expr(
    expr: &Expr,
    variables: &HashMap<String, Option<i64>>,
    signatures: &HashMap<String, usize>,
) -> Result<Option<i64>, SemanticError> {
    match expr {
        Expr::Integer(value) => Ok(Some(*value)),
        Expr::Name(name) => variables.get(name).copied().ok_or_else(|| {
            failure("unknown_name", format!("undeclared scalar '{name}'"))
        }),
        Expr::Call { name, args } => {
            let Some(expected) = signatures.get(name) else {
                return Err(failure(
                    "unknown_name",
                    format!("unknown function '{name}'"),
                ));
            };
            if args.len() != *expected {
                return Err(failure(
                    "call_arity",
                    format!(
                        "{name}: expected {expected} parameters, received {}",
                        args.len()
                    ),
                ));
            }
            // Evaluate/validate ALL arguments even if the callee would ignore
            // them. Never treat a call as a compile-time pure constant.
            for arg in args {
                validate_expr(arg, variables, signatures)?;
            }
            Ok(None)
        }
        Expr::Add(a, b) => {
            checked_binary(a, b, variables, signatures, "addition", i64::checked_add)
        }
        Expr::Subtract(a, b) => {
            checked_binary(a, b, variables, signatures, "subtraction", i64::checked_sub)
        }
        Expr::Multiply(a, b) => {
            checked_binary(a, b, variables, signatures, "multiplication", i64::checked_mul)
        }
        Expr::Negate(expr) => match validate_expr(expr, variables, signatures)? {
            Some(value) => value.checked_neg().map(Some).ok_or_else(|| {
                failure("checked_overflow", "statically proven signed i64 negation overflow")
            }),
            None => Ok(None),
        },
    }
}

fn checked_binary(
    a: &Expr,
    b: &Expr,
    variables: &HashMap<String, Option<i64>>,
    signatures: &HashMap<String, usize>,
    operator: &'static str,
    operation: fn(i64, i64) -> Option<i64>,
) -> Result<Option<i64>, SemanticError> {
    // Validate each subtree BEFORE discarding a dynamic result: a nested
    // provable overflow is never made safe by a later multiplication by zero.
    let lhs = validate_expr(a, variables, signatures)?;
    let rhs = validate_expr(b, variables, signatures)?;
    match (lhs, rhs) {
        (Some(lhs), Some(rhs)) => operation(lhs, rhs).map(Some).ok_or_else(|| {
            failure(
                "checked_overflow",
                format!("statically proven signed i64 {operator} overflow"),
            )
        }),
        _ => Ok(None),
    }
}
pub fn lower(program: Program) -> Result<Module, SemanticError> {
    lower_with_ref_layout(program, ManagedRefLayout::default())
}

pub fn lower_with_ref_layout(
    program: Program,
    managed_refs: ManagedRefLayout,
) -> Result<Module, SemanticError> {
    let mut signatures = HashMap::new();
    for function in &program.functions {
        if signatures
            .insert(function.name.clone(), function.params.len())
            .is_some()
        {
            return Err(failure(
                "duplicate_declaration",
                format!("duplicate function '{}'", function.name),
            ));
        }
    }
    let mut functions = Vec::with_capacity(program.functions.len());
    for function in program.functions {
        let mut vars: HashMap<String, Option<i64>> = HashMap::new();
        for param in &function.params {
            if vars.insert(param.clone(), None).is_some() {
                return Err(failure(
                    "duplicate_declaration",
                    format!("duplicate parameter '{param}'"),
                ));
            }
        }
        let mut locals = Vec::new();
        for local in function.locals {
            let constant = validate_expr(&local.value, &vars, &signatures)?;
            if vars.insert(local.name.clone(), constant).is_some() {
                return Err(failure(
                    "duplicate_declaration",
                    format!("duplicate local '{}'", local.name),
                ));
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
        managed_refs,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    fn check(source: &str) -> Result<Module, SemanticError> {
        lower(crate::frontend::parse(source).unwrap())
    }
    #[test]
    fn managed_reference_layout_is_preserved_in_mir() {
        let program = crate::frontend::parse("pub fnc main(): int { return 42; }").unwrap();
        let module =
            lower_with_ref_layout(program, ManagedRefLayout::native()).expect("valid scalar MIR");
        assert_eq!(module.managed_refs, ManagedRefLayout::native());
    }

    #[test]
    fn duplicate_function_rejected() {
        assert_eq!(
            check("fnc f(): int { return 1; } fnc f(): int { return 2; }")
                .unwrap_err()
                .class,
            "duplicate_declaration"
        );
    }
    #[test]
    fn forward_call_admitted() {
        check("pub fnc main(): int { return times(7,6); } fnc times(int a,int b): int { return a*b; }").unwrap();
    }
    #[test]
    fn unknown_name_rejected() {
        assert_eq!(
            check("pub fnc main(): int { return neverDeclared; }")
                .unwrap_err()
                .class,
            "unknown_name"
        );
    }
    #[test]
    fn call_arity_rejected() {
        assert_eq!(
            check(
                "fnc add(int a,int b): int { return a+b; } pub fnc main(): int { return add(1); }"
            )
            .unwrap_err()
            .class,
            "call_arity"
        );
    }
    #[test]
    fn shadowed_local_rejected() {
        assert_eq!(
            check("fnc f(int a): int { const a: int = 3; return a; }")
                .unwrap_err()
                .class,
            "duplicate_declaration"
        );
    }
    #[test]
    fn local_binding_order_enforced() {
        assert_eq!(
            check("fnc f(): int { const a: int = b; return a; }")
                .unwrap_err()
                .class,
            "unknown_name"
        );
    }
}
