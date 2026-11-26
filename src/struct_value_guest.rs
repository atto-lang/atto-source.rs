//! Bounded executable attached-struct profile: one file-local nominal owner,
//! scalar fields and parameters, read-only single-return methods and
//! straight-line main. Values live in independent typed scalar locals.
//! Never use the identity-bearing class ABI or an interpreter.
use crate::{ast::*, parser::{Parser, StructWithTarget, MAX_PARAMS}, lexer::Token, Diagnostic, Pos};
use std::collections::{BTreeMap, BTreeSet};

type R<T> = Result<T, Diagnostic>;
type Slots = BTreeMap<String, String>;

fn denied(pos: Pos, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(pos, "E_UNSUPPORTED", message)
}
fn variable(name: &str, pos: Pos) -> Expr {
    Expr { kind: ExprKind::Var(name.to_owned()), pos }
}
fn binding(name: String, ty: Type, init: Expr, pos: Pos) -> Stmt {
    Stmt::Bind { kind: BindKind::Val, name, ty: Some(ty), init, pos }
}
fn internal(kind: &str, index: usize, slot: usize) -> String {
    // '$' cannot appear in a source identifier, so source cannot capture or
    // call compiler-generated local and method symbols.
    format!("$struct_{kind}_{index}_{slot}")
}

// A constructor or method expression may only read a scalar parameter or
// a proved initialized self field. No self escape, dynamic call, ownership
// operation, allocation, closure, effect, unchecked projection or mutation.
fn member_expr(e: &Expr, params: &Slots, fields: &Slots) -> R<Expr> {
    let pos = e.pos;
    let kind = match &e.kind {
        ExprKind::Int(n) => ExprKind::Int(*n),
        ExprKind::Bool(b) => ExprKind::Bool(*b),
        ExprKind::Var(n) => return params.get(n).map(|s| variable(s, pos))
            .ok_or_else(|| denied(pos, "unbound scalar member parameter")),
        ExprKind::Member(base, name) if matches!(&base.kind, ExprKind::SelfRef) => {
            return fields.get(name).map(|s| variable(s, pos))
                .ok_or_else(|| denied(pos, "unknown or uninitialized struct field"));
        }
        ExprKind::Unary(op, a) => ExprKind::Unary(*op, Box::new(member_expr(a, params, fields)?)),
        ExprKind::Binary(op, a, b) => ExprKind::Binary(*op,
            Box::new(member_expr(a, params, fields)?), Box::new(member_expr(b, params, fields)?)),
        ExprKind::Ternary(c, a, b) => ExprKind::Ternary(
            Box::new(member_expr(c, params, fields)?), Box::new(member_expr(a, params, fields)?),
            Box::new(member_expr(b, params, fields)?)),
        _ => return Err(denied(pos, "attached member requires a closed scalar expression")),
    };
    Ok(Expr { kind, pos })
}

struct MethodEntry {
    function: String,
    arity: usize,
}
struct LocalValue {
    fields: Vec<String>,
}
struct MainLower<'a> {
    decl: &'a StructWithTarget,
    methods: &'a BTreeMap<(String, usize), MethodEntry>,
    type_names: &'a [(String, Pos)],
    names: BTreeSet<String>,
    scalars: BTreeSet<String>,
    values: BTreeMap<String, LocalValue>,
    next: usize,
}

impl MainLower<'_> {
    fn expr(&self, e: &Expr) -> R<Expr> {
        let pos = e.pos;
        let kind = match &e.kind {
            ExprKind::Int(n) => ExprKind::Int(*n),
            ExprKind::Bool(b) => ExprKind::Bool(*b),
            ExprKind::Var(n) if self.scalars.contains(n) => ExprKind::Var(n.clone()),
            ExprKind::Unary(op, a) => ExprKind::Unary(*op, Box::new(self.expr(a)?)),
            ExprKind::Binary(op, a, b) => ExprKind::Binary(*op, Box::new(self.expr(a)?),
                Box::new(self.expr(b)?)),
            ExprKind::Ternary(c, a, b) => ExprKind::Ternary(Box::new(self.expr(c)?),
                Box::new(self.expr(a)?), Box::new(self.expr(b)?)),
            ExprKind::MethodCall(base, name, args) => {
                let ExprKind::Var(owner) = &base.kind else {
                    return Err(denied(pos, "attached method requires a named initialized local struct"));
                };
                let value = self.values.get(owner).ok_or_else(||
                    denied(pos, "method receiver is not a local value of the declared nominal struct"))?;
                let method = self.methods.get(&(name.clone(), args.len())).ok_or_else(||
                    Diagnostic::new(pos, "E_ARITY", "no matching attached struct method"))?;
                if value.fields.len() + args.len() != method.arity {
                    return Err(denied(pos, "invalid flattened method arity"));
                }
                let mut actual: Vec<_> = value.fields.iter().map(|s| variable(s, pos)).collect();
                for arg in args { actual.push(self.expr(arg)?); }
                ExprKind::Call(method.function.clone(), actual)
            }
            _ => return Err(denied(pos, "scalar struct profile does not admit this expression or value escape")),
        };
        Ok(Expr { kind, pos })
    }

    fn construct(&mut self, name: &str, args: &[Expr], pos: Pos, out: &mut Vec<Stmt>) -> R<()> {
        // This profile deliberately rejects same-arity overloads even when a
        // future full nominal type resolver could distinguish their types.
        let mut candidates = self.decl.constructors.iter().filter(|(_, c)| c.params.len() == args.len());
        let (_, ctor) = candidates.next().ok_or_else(||
            Diagnostic::new(pos, "E_ARITY", "no matching attached struct constructor"))?;
        if candidates.next().is_some() {
            return Err(denied(pos, "same-arity constructor overload requires nominal overload resolution"));
        }
        if ctor.params.iter().any(|p| p.mutable) {
            return Err(denied(pos, "mutable constructor parameter outside scalar profile"));
        }
        let id = self.next;
        self.next += 1;
        let mut params = Slots::new();
        let mut initialized = Slots::new();
        // Every argument is evaluated exactly once, in source order, before
        // any constructor field initializer. Ordinary typed checker checks
        // all conversions and the checked arithmetic/trap semantics.
        for (i, (param, arg)) in ctor.params.iter().zip(args).enumerate() {
            let local = internal("arg", id, i);
            out.push(binding(local.clone(), param.ty, self.expr(arg)?, arg.pos));
            params.insert(param.name.clone(), local);
        }
        for stmt in &ctor.body {
            let Stmt::FieldAssign { target, field, value, pos: at } = stmt else {
                return Err(denied(pos, "constructor is not a proved straight-line scalar initializer"));
            };
            if !matches!(&target.kind, ExprKind::SelfRef) || initialized.contains_key(field) {
                return Err(denied(*at, "invalid constructor field write"));
            }
            let (i, (_, ty)) = self.decl.fields.iter().enumerate().find(|(_, (n, _))| n == field)
                .ok_or_else(|| denied(*at, "unknown constructor field"))?;
            let local = internal("field", id, i);
            let init = member_expr(value, &params, &initialized)?;
            out.push(binding(local.clone(), *ty, init, *at));
            initialized.insert(field.clone(), local);
        }
        let fields = self.decl.fields.iter().map(|(n, _)| initialized.get(n).cloned()
            .ok_or_else(|| denied(pos, "constructor leaves uninitialized field"))).collect::<R<Vec<_>>>()?;
        self.values.insert(name.to_owned(), LocalValue { fields });
        Ok(())
    }

    fn validate_value_binding(&self, ty: Option<Type>, kind: BindKind, pos: Pos) -> R<()> {
        if kind == BindKind::Const {
            return Err(denied(pos, "runtime struct value is not a compile-time const"));
        }
        if let Some(t) = ty {
            let matches_owner = match t {
                Type::Named(index) => self.type_names.get(index as usize)
                    .is_some_and(|(name, _)| name == &self.decl.name),
                _ => false,
            };
            if !matches_owner {
                return Err(Diagnostic::new(pos, "E_TYPE", "struct binding annotation must name its exact nominal owner"));
            }
        }
        Ok(())
    }

    // Flat scalar values are copied into distinct immutable locals, never
    // aliased or passed through the identity-bearing class copy machinery.
    fn copy_value(&mut self, name: &str, source: &str, pos: Pos, out: &mut Vec<Stmt>) -> R<()> {
        let value = self.values.get(source).ok_or_else(|| denied(pos, "copy requires a local scalar struct"))?;
        if value.fields.len() != self.decl.fields.len() {
            return Err(denied(pos, "invalid struct field plan"));
        }
        let id = self.next;
        self.next += 1;
        let mut fields = Vec::with_capacity(value.fields.len());
        for (i, (old, (_, ty))) in value.fields.iter().zip(&self.decl.fields).enumerate() {
            let local = internal("copy", id, i);
            out.push(binding(local.clone(), *ty, variable(old, pos), pos));
            fields.push(local);
        }
        self.values.insert(name.to_owned(), LocalValue { fields });
        Ok(())
    }

    fn body(&mut self, body: &[Stmt]) -> R<Vec<Stmt>> {
        let mut out = Vec::new();
        for stmt in body {
            match stmt {
                Stmt::Bind { kind, name, ty, init, pos } => {
                    if name == &self.decl.name || !self.names.insert(name.clone()) {
                        return Err(Diagnostic::new(*pos, "E_DUPLICATE", "duplicate local or nominal owner shadow"));
                    }
                    if let ExprKind::Call(owner, args) = &init.kind {
                        if owner == &self.decl.name {
                            self.validate_value_binding(*ty, *kind, *pos)?;
                            self.construct(name, args, *pos, &mut out)?;
                            continue;
                        }
                    }
                    let copy_source = match &init.kind {
                        ExprKind::Var(source) if self.values.contains_key(source) => Some(source.as_str()),
                        ExprKind::Ownership(OwnershipOp::Copy, false, inner) => match &inner.kind {
                            ExprKind::Var(source) if self.values.contains_key(source) => Some(source.as_str()),
                            _ => None,
                        },
                        _ => None,
                    };
                    if let Some(source) = copy_source {
                        self.validate_value_binding(*ty, *kind, *pos)?;
                        self.copy_value(name, source, *pos, &mut out)?;
                        continue;
                    }
                    if ty.is_some_and(|t| !matches!(t, Type::Int | Type::Bool)) {
                        return Err(denied(*pos, "non-scalar local type"));
                    }
                    let init = self.expr(init)?;
                    out.push(Stmt::Bind { kind: *kind, name: name.clone(), ty: *ty, init, pos: *pos });
                    self.scalars.insert(name.clone());
                }
                Stmt::Expr(Expr { kind: ExprKind::Call(name, args), pos })
                    if name == "stdio.println" && args.len() == 1 && !self.names.contains("stdio") => {
                    out.push(Stmt::Expr(Expr { kind: ExprKind::Call(name.clone(), vec![self.expr(&args[0])?]),
                        pos: *pos }));
                }
                Stmt::Return(Some(e), pos) => out.push(Stmt::Return(Some(self.expr(e)?), *pos)),
                _ => return Err(denied(self.decl.pos, "only immutable scalar/struct local construction, scalar println and return are admitted")),
            }
        }
        if self.values.is_empty() {
            return Err(denied(self.decl.pos, "scalar struct guest requires a local constructor invocation"));
        }
        Ok(out)
    }
}

pub(crate) fn lower(tokens: Vec<Token>) -> R<Program> {
    let mut parser = Parser::new(tokens);
    let decl = parser.parse_struct_with_target()?;
    let mut program = parser.parse_program()?;
    if !program.classes.is_empty() || !program.enums.is_empty() || !program.actors.is_empty()
        || program.actor_interface.is_some() || !program.array_types.is_empty()
        || program.funcs.len() != 1 {
        return Err(denied(decl.pos, "scalar struct guest requires exactly one standalone main and one nominal owner"));
    }
    let main = &program.funcs[0];
    if main.name != "main" || !main.params.is_empty() || main.ret != Type::Int
        || main.is_generator || main.is_pure || main.declared_effects.is_some() {
        return Err(denied(main.pos, "scalar struct guest requires plain main(): int"));
    }
    if decl.methods.len() > 64 || decl.fields.len() > MAX_PARAMS || decl.constructors.is_empty() {
        return Err(denied(decl.pos, "scalar struct guest declaration exceeds bounded profile"));
    }
    let mut methods = BTreeMap::new();
    let mut lowered = Vec::new();
    for (i, m) in decl.methods.iter().enumerate() {
        if m.is_static || m.params.iter().any(|p| p.mutable) || m.ret == Type::Void
            || decl.fields.len() + m.params.len() > MAX_PARAMS {
            return Err(denied(m.pos, "attached method outside scalar readonly profile"));
        }
        let [Stmt::Return(Some(e), ret_pos)] = m.body.as_slice() else {
            return Err(denied(m.pos, "readonly method requires exactly one scalar return expression"));
        };
        let mut params = Slots::new();
        let mut fields = Slots::new();
        let mut flat = Vec::new();
        for (j, (name, ty)) in decl.fields.iter().enumerate() {
            let local = internal("receiver", i, j);
            fields.insert(name.clone(), local.clone());
            flat.push(Param { name: local, ty: *ty, mutable: false, pos: m.pos });
        }
        for (j, p) in m.params.iter().enumerate() {
            let local = internal("param", i, j);
            params.insert(p.name.clone(), local.clone());
            flat.push(Param { name: local, ty: p.ty, mutable: false, pos: p.pos });
        }
        let function = internal("method", i, 0);
        let entry = MethodEntry { function: function.clone(), arity: flat.len() };
        if methods.insert((m.name.clone(), m.params.len()), entry).is_some() {
            return Err(denied(m.pos, "same-arity method overload requires nominal overload resolution"));
        }
        lowered.push(Func { name: function, pos: m.pos, is_generator: false, is_pure: false,
            declared_effects: None, params: flat, ret: m.ret,
            body: vec![Stmt::Return(Some(member_expr(e, &params, &fields)?), *ret_pos)] });
    }
    let mut lower = MainLower { decl: &decl, methods: &methods, type_names: &program.type_names, names: BTreeSet::new(),
        scalars: BTreeSet::new(), values: BTreeMap::new(), next: 0 };
    program.funcs[0].body = lower.body(&program.funcs[0].body)?;
    program.funcs.extend(lowered);
    Ok(program)
}
