//! Validated scalar MIR. This is intentionally an Oreslang IR, not Cranelift IR.
use crate::{
    frontend::{Export, ExportScope, Expr, Program},
    ref_layout::ManagedRefLayout,
};
use std::collections::{HashMap, HashSet};
use std::fmt;

#[derive(Clone, Debug)]
pub struct Module {
    pub functions: Vec<Function>,
    pub managed_refs: ManagedRefLayout,
    pub exports: Vec<Export>,
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
// Mirrors the protected ADT/collection/loan identity subset in src/prelude.rs.
// The scalar backend cannot lower these identities as ordinary user routines.
fn protected_identity(name: &str) -> bool {
    matches!(
        name,
        "Option"
            | "Some"
            | "None"
            | "Result"
            | "Ok"
            | "Err"
            | "Array"
            | "FixedArray"
            | "Vector"
            | "List"
            | "FixedList"
            | "Slice"
            | "Tuple"
            | "Map"
            | "Set"
            | "ReadLoan"
            | "WriteLoan"
    )
}

fn validate_expr(
    expr: &Expr,
    variables: &HashSet<String>,
    signatures: &HashMap<String, usize>,
) -> Result<(), SemanticError> {
    match expr {
        Expr::Integer(_) => Ok(()),
        Expr::Name(name) => {
            if variables.contains(name) {
                Ok(())
            } else {
                Err(failure(
                    "unknown_name",
                    format!("undeclared scalar '{name}'"),
                ))
            }
        }
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
            for arg in args {
                validate_expr(arg, variables, signatures)?;
            }
            Ok(())
        }
        Expr::Add(a, b) | Expr::Subtract(a, b) | Expr::Multiply(a, b) => {
            validate_expr(a, variables, signatures)?;
            validate_expr(b, variables, signatures)
        }
        Expr::Negate(expr) => validate_expr(expr, variables, signatures),
    }
}
// Closed, uniformly unsigned MIR. The underlying scalar syntax and MIR store
// i64 bit patterns; the wrapper fixes the mathematical type for every value.
// No public conversion exposes it to signed codegen or signed facts exporters.
pub(crate) struct UnsignedModule(Module);
impl UnsignedModule {
    pub(crate) fn inner(&self) -> &Module { &self.0 }
}
pub(crate) fn lower_unsigned(source: &str) -> Result<UnsignedModule, String> {
    let ast = crate::frontend::parse_unsigned(source).map_err(|e| e.to_string())?;
    let module = lower(ast).map_err(|e| e.to_string())?;
    let entry = module.functions.iter().find(|f| f.symbol == "main").ok_or("unsigned main missing")?;
    if !entry.exported || !entry.params.is_empty() || module.functions.iter().any(|f| f.symbol == "atto_guest_main") {
        return Err("unsigned profile requires a public zero-argument main and no reserved observer symbol".into());
    }
    // This reuses the existing bounded expression traversal only, not its
    // signed effect facts as a certificate.
    let _ = crate::effects::export_effect_ir_checked(&module)?;
    validate_unsigned_work(&module)?;
    Ok(UnsignedModule(module))
}

// Match the JS unsigned candidate's closed acyclic execution profile. Charge
// every syntactic expression and each invocation, including unused locals
// and functions. Postorder memoization avoids expanding repeated callees.
fn validate_unsigned_work(module: &Module) -> Result<(), String> {
    const MAX_FUNCTIONS: usize = 32;
    const MAX_EXPANDED: usize = 10_000;
    if module.functions.len() > MAX_FUNCTIONS {
        return Err("unsigned function budget exceeded".into());
    }
    let index: HashMap<&str, usize> = module.functions.iter().enumerate()
        .map(|(i, f)| (f.symbol.as_str(), i)).collect();
    let mut costs = vec![None; module.functions.len()];
    let mut active = vec![false; module.functions.len()];
    fn cost(i: usize, module: &Module, index: &HashMap<&str, usize>,
        costs: &mut [Option<usize>], active: &mut [bool]) -> Result<usize, String> {
        if active[i] { return Err("recursive unsigned call graph".into()); }
        if let Some(n) = costs[i] { return Ok(n); }
        active[i] = true;
        let function = &module.functions[i];
        let mut pending: Vec<&Expr> = function.locals.iter().map(|(_, e)| e).collect();
        pending.push(&function.return_value);
        let mut total = 0usize;
        while let Some(expr) = pending.pop() {
            total += 1;
            match expr {
                Expr::Integer(_) | Expr::Name(_) => {}
                Expr::Negate(_) => return Err("signed negation in unsigned MIR".into()),
                Expr::Add(a,b) | Expr::Subtract(a,b) | Expr::Multiply(a,b) => {
                    pending.push(a); pending.push(b);
                }
                Expr::Call { name, args } => {
                    let target = *index.get(name.as_str()).ok_or("unknown unsigned callee")?;
                    total += cost(target, module, index, costs, active)?;
                    pending.extend(args.iter());
                }
            }
            if total > MAX_EXPANDED { return Err("unsigned expanded work budget exceeded".into()); }
        }
        active[i] = false;
        costs[i] = Some(total);
        Ok(total)
    }
    for i in 0..module.functions.len() { cost(i, module, &index, &mut costs, &mut active)?; }
    Ok(())
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
        if protected_identity(&function.name) {
            return Err(failure(
                "reserved_collection_name",
                format!(
                    "compiler-owned identity '{}' cannot name a scalar function",
                    function.name
                ),
            ));
        }
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
    let mut export_names = HashSet::new();
    for export in &program.exports {
        if !export_names.insert(export.name.as_str()) {
            return Err(failure("duplicate_export", "duplicate MIR export"));
        }
        let function = program.functions.iter().find(|f| f.name == export.name)
            .ok_or_else(|| failure("export_unresolved", "MIR export must name a local function"))?;
        if (export.scope == ExportScope::Public && !function.exported)
            || (export.scope == ExportScope::Package && (function.exported || function.name == "main"))
        {
            return Err(failure("export_visibility_conflict", "inconsistent MIR export linkage"));
        }
    }
    let mut functions = Vec::with_capacity(program.functions.len());
    for function in program.functions {
        let mut vars = HashSet::new();
        for param in &function.params {
            if !vars.insert(param.clone()) {
                return Err(failure(
                    "duplicate_declaration",
                    format!("duplicate parameter '{param}'"),
                ));
            }
        }
        let mut locals = Vec::new();
        for local in function.locals {
            validate_expr(&local.value, &vars, &signatures)?;
            if !vars.insert(local.name.clone()) {
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
        exports: program.exports,
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
