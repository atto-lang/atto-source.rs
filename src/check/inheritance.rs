//! Compile-time inheritance lowering for the bounded class/trait subset.
//! A concrete method is never selected by parent order: identical inherited
//! declarations are deduplicated by origin, competing origins need an override.
//! This intentionally does not claim dynamic upcasts or virtual tables.

use std::collections::{BTreeMap, BTreeSet};
use crate::ast::{ClassDecl, ContractDecl, Field, Method, Program, Type};
use crate::diag::{Diagnostic, Pos};

type R<T> = Result<T, Diagnostic>;
const MAX_ANCESTRY_DEPTH: usize = 32;
const MAX_EXPANDED_MEMBERS: usize = 256;

#[derive(Clone)]
struct Pick<T> {
    origin: String,
    value: T,
}

#[derive(Clone, Default)]
struct Expanded {
    fields: Vec<Pick<Field>>,
    methods: Vec<Pick<Method>>,
    interfaces: BTreeSet<String>,
}

fn fail(pos: Pos, code: &'static str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(pos, code, message)
}

fn type_name(ty: Type, program: &Program) -> String {
    match ty {
        Type::Named(n) | Type::NamedRef(n) | Type::Callable(n) =>
            program.type_names.get(n as usize).map(|v| v.0.clone())
                .unwrap_or_else(|| format!("invalid-type-{n}")),
        other => format!("{other:?}"),
    }
}

fn compatible(a: &Method, b: &Method, program: &Program) -> bool {
    !a.is_static && !b.is_static && a.params.len() == b.params.len()
        && type_name(a.ret, program) == type_name(b.ret, program)
        && a.params.iter().zip(&b.params).all(|(x, y) =>
            type_name(x.ty, program) == type_name(y.ty, program))
}

fn merge_pick<T: Clone>(
    into: &mut BTreeMap<String, Vec<Pick<T>>>, name: &str, picked: Pick<T>,
) {
    into.entry(name.to_owned()).or_default().push(picked);
}

fn distinct<T: Clone>(list: Vec<Pick<T>>) -> Vec<Pick<T>> {
    let mut seen = BTreeSet::new();
    list.into_iter().filter(|p| seen.insert(p.origin.clone())).collect()
}

struct Resolver<'a> {
    program: &'a Program,
    classes: BTreeMap<String, usize>,
    contracts: BTreeMap<String, &'a ContractDecl>,
    visiting: Vec<bool>,
    cache: Vec<Option<Expanded>>,
}

impl Resolver<'_> {
    fn expand(&mut self, index: usize, depth: usize) -> R<Expanded> {
        let class = &self.program.classes[index];
        if depth > MAX_ANCESTRY_DEPTH {
            return Err(fail(class.pos, "E_INHERITANCE_DEPTH",
                "class inheritance exceeds the bounded ancestry depth"));
        }
        if self.visiting[index] {
            return Err(fail(class.pos, "E_INHERITANCE_CYCLE",
                format!("inheritance cycle involving '{}'", class.name)));
        }
        if let Some(cached) = &self.cache[index] {
            return Ok(cached.clone());
        }
        self.visiting[index] = true;
        let result = self.expand_unchecked(index, depth);
        self.visiting[index] = false;
        if let Ok(ref value) = result { self.cache[index] = Some(value.clone()); }
        result
    }

    fn expand_unchecked(&mut self, index: usize, depth: usize) -> R<Expanded> {
        let class = &self.program.classes[index];
        let mut fields: BTreeMap<String, Vec<Pick<Field>>> = BTreeMap::new();
        let mut methods: BTreeMap<String, Vec<Pick<Method>>> = BTreeMap::new();
        let mut interfaces = BTreeSet::new();
        for (base, pos) in &class.bases {
            let Some(&parent_index) = self.classes.get(base) else {
                return Err(fail(*pos, "E_UNKNOWN_TYPE",
                    format!("'{}' is not a concrete class base", base)));
            };
            // No implicit constructor chain: the v1 lowering supports parents
            // with field initializers, but not user-defined base constructors.
            if self.program.classes[parent_index].ctor.is_some() {
                return Err(fail(*pos, "E_UNSUPPORTED",
                    format!("inherited constructor '{}' requires explicit base initialization", base)));
            }
            let inherited = self.expand(parent_index, depth + 1)?;
            interfaces.extend(inherited.interfaces);
            for f in inherited.fields {
                if !f.value.public {
                    return Err(fail(*pos, "E_UNSUPPORTED",
                        format!("inheriting private field '{}.{}' is not yet supported",
                            f.origin, f.value.name)));
                }
                let name = f.value.name.clone();
                merge_pick(&mut fields, &name, f);
            }
            for m in inherited.methods {
                if !m.value.public {
                    return Err(fail(*pos, "E_UNSUPPORTED",
                        format!("inheriting private method '{}.{}' is not yet supported",
                            m.origin, m.value.name)));
                }
                let name = m.value.name.clone();
                merge_pick(&mut methods, &name, m);
            }
        }
        // Both contracts and class bases are resolved nominally. A trait
        // supplies its concrete method once; an interface only adds obligations.
        for (name, pos) in &class.implements {
            let Some(contract) = self.contracts.get(name) else {
                return Err(fail(*pos, "E_UNKNOWN_TYPE",
                    format!("'{}' is not a known trait or interface", name)));
            };
            if contract.interface {
                interfaces.insert(name.clone());
            } else {
                for method in &contract.methods {
                    merge_pick(&mut methods, &method.name, Pick {
                        origin: contract.name.clone(), value: method.clone(),
                    });
                }
            }
        }

        let mut out = Expanded { interfaces, ..Expanded::default() };
        for field in &class.fields {
            if fields.contains_key(&field.name) {
                return Err(fail(field.pos, "E_AMBIGUOUS_MEMBER",
                    format!("field '{}' shadows an inherited field; explicit field resolution is not implemented",
                        field.name)));
            }
            out.fields.push(Pick { origin: class.name.clone(), value: field.clone() });
        }
        for (name, bucket) in fields {
            let unique = distinct(bucket);
            if unique.len() != 1 {
                return Err(fail(class.pos, "E_AMBIGUOUS_MEMBER",
                    format!("inherited field '{name}' has multiple declarations")));
            }
            out.fields.push(unique.into_iter().next().unwrap());
        }
        let mut declared = BTreeSet::new();
        for method in &class.methods {
            if !declared.insert(method.name.clone()) {
                return Err(fail(method.pos, "E_DUPLICATE",
                    format!("duplicate method '{}.{}'", class.name, method.name)));
            }
            if method.is_static {
                if method.override_member {
                    return Err(fail(method.pos, "E_OVERRIDE",
                        "static methods cannot override instance methods"));
                }
                out.methods.push(Pick { origin: class.name.clone(), value: method.clone() });
                continue;
            }
            let inherited = methods.remove(&method.name).unwrap_or_default();
            if inherited.is_empty() && method.override_member {
                return Err(fail(method.pos, "E_OVERRIDE",
                    format!("'{}' has no inherited method to override", method.name)));
            }
            if !inherited.is_empty() && !method.override_member {
                return Err(fail(method.pos, "E_OVERRIDE_REQUIRED",
                    format!("'{}' must use override to replace an inherited method", method.name)));
            }
            for candidate in distinct(inherited) {
                if !compatible(method, &candidate.value, self.program) {
                    return Err(fail(method.pos, "E_OVERRIDE_SIGNATURE",
                        format!("'{}' must preserve every inherited method signature", method.name)));
                }
                if candidate.value.public && !method.public {
                    return Err(fail(method.pos, "E_OVERRIDE_VISIBILITY",
                        format!("'{}' cannot reduce inherited method visibility", method.name)));
                }
            }
            out.methods.push(Pick { origin: class.name.clone(), value: method.clone() });
        }
        for (name, bucket) in methods {
            let unique = distinct(bucket);
            if unique.len() != 1 {
                return Err(fail(class.pos, "E_AMBIGUOUS_MEMBER",
                    format!("ambiguous method '{name}' inherited from multiple origins; add an explicit override")));
            }
            out.methods.push(unique.into_iter().next().unwrap());
        }

        // Ensure interface requirements are satisfied by a concrete instance
        // implementation, including inherited methods and trait defaults.
        for interface in &out.interfaces {
            let contract = self.contracts[interface];
            for required in &contract.methods {
                let Some(found) = out.methods.iter().find(|m|
                    !m.value.is_static && m.value.name == required.name) else {
                    return Err(fail(class.pos, "E_INTERFACE",
                        format!("class '{}' does not implement '{}.{}'",
                            class.name, interface, required.name)));
                };
                if !compatible(&found.value, required, self.program) || !found.value.public {
                    return Err(fail(class.pos, "E_INTERFACE",
                        format!("class '{}' has an incompatible method for '{}.{}'",
                            class.name, interface, required.name)));
                }
            }
        }
        if out.fields.len() + out.methods.len() > MAX_EXPANDED_MEMBERS {
            return Err(fail(class.pos, "E_SOURCE_LIMIT",
                "expanded inherited members exceed the per-class limit"));
        }
        Ok(out)
    }
}

/// Lower inheritance into concrete class layouts and statically callable methods.
/// No runtime parent-selection precedence is encoded or inferred.
pub fn lower(input: &Program) -> R<Program> {
    let mut names = BTreeMap::<String, Pos>::new();
    for (name, pos) in input.classes.iter().map(|c| (&c.name, c.pos))
        .chain(input.contracts.iter().map(|c| (&c.name, c.pos)))
        .chain(input.actors.iter().map(|a| (&a.name, a.pos)))
        .chain(input.enums.iter().map(|e| (&e.name, e.pos))) {
        if names.insert(name.clone(), pos).is_some() {
            return Err(fail(pos, "E_DUPLICATE",
                format!("type '{name}' is declared more than once")));
        }
    }
    for func in &input.funcs {
        if names.contains_key(&func.name) {
            return Err(fail(func.pos, "E_DUPLICATE",
                format!("function '{}' conflicts with a type name", func.name)));
        }
    }
    if input.contracts.is_empty() && input.classes.iter().all(|c|
        c.bases.is_empty() && c.implements.is_empty() &&
        c.methods.iter().all(|m| !m.override_member)) {
        return Ok(input.clone());
    }
    let classes = input.classes.iter().enumerate()
        .map(|(i, c)| (c.name.clone(), i)).collect();
    let contracts = input.contracts.iter().map(|c| (c.name.clone(), c)).collect();
    let n = input.classes.len();
    let mut resolver = Resolver { program: input, classes, contracts,
        visiting: vec![false; n], cache: vec![None; n] };
    let mut program = input.clone();
    for (i, class) in program.classes.iter_mut().enumerate() {
        let expanded = resolver.expand(i, 0)?;
        class.fields = expanded.fields.into_iter().map(|f| f.value).collect();
        class.methods = expanded.methods.into_iter().map(|mut m| {
            // The override has already been checked. A copied implementation
            // is now an ordinary concrete method on this exact receiver type.
            m.value.override_member = false;
            m.value
        }).collect();
        class.bases.clear();
        class.implements.clear();
    }
    program.contracts.clear();
    Ok(program)
}
