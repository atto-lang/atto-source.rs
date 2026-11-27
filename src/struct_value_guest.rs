//! Bounded executable attached-struct profile: one file-local nominal owner,
//! scalar fields and parameters, readonly returns and bounded mutable methods,
//! straight-line main. Values live in independent typed scalar locals.
//! Never use the identity-bearing class ABI or an interpreter.
use crate::{ast::*, parser::{Parser, StructWithTarget, MAX_PARAMS}, lexer::Token, Diagnostic, Pos};
use std::collections::{BTreeMap, BTreeSet};

type R<T> = Result<T, Diagnostic>;
type Slots = BTreeMap<String, String>;

// Bound compiler-generated AST growth before cloning constructor/mutator
// bodies at call sites. A small source must not amplify into unbounded
// compiler memory or work. The existing checker/emitter limits remain in force.
const MAX_STRUCT_EXPANSION_NODES: usize = 65_536;

fn expression_cost(e: &Expr) -> usize {
    let children = match &e.kind {
        ExprKind::Unary(_, a) => expression_cost(a),
        ExprKind::Binary(_, a, b) => expression_cost(a).saturating_add(expression_cost(b)),
        ExprKind::Ternary(a, b, c) => expression_cost(a).saturating_add(expression_cost(b))
            .saturating_add(expression_cost(c)),
        ExprKind::Member(base, _) => expression_cost(base),
        _ => 0,
    };
    1usize.saturating_add(children)
}

fn body_cost(body: &[Stmt]) -> usize {
    body.iter().fold(0usize, |sum, stmt| {
        let children = match stmt {
            Stmt::FieldAssign { target, value, .. } =>
                expression_cost(target).saturating_add(expression_cost(value)),
            Stmt::If { arms, otherwise, .. } => arms.iter().fold(
                otherwise.as_ref().map_or(0, |b| body_cost(b)), |n, (cond, b)|
                n.saturating_add(expression_cost(cond)).saturating_add(body_cost(b))),
            Stmt::Return(Some(e), _) => expression_cost(e),
            _ => 0,
        };
        sum.saturating_add(1).saturating_add(children)
    })
}

fn charge_expansion(used: &mut usize, extra: usize, pos: Pos) -> R<()> {
    let next = used.saturating_add(extra);
    if next > MAX_STRUCT_EXPANSION_NODES {
        return Err(Diagnostic::new(pos, "E_SOURCE_LIMIT", "attached struct expansion node budget exceeded"));
    }
    *used = next;
    Ok(())
}

fn denied(pos: Pos, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(pos, "E_UNSUPPORTED", message)
}
fn variable(name: &str, pos: Pos) -> Expr {
    Expr { kind: ExprKind::Var(name.to_owned()), pos }
}
fn binding(name: String, ty: Type, init: Expr, pos: Pos) -> Stmt {
    Stmt::Bind { kind: BindKind::Val, name, ty: Some(ty), init, pos }
}
fn field_binding(name: String, ty: Type, init: Expr, pos: Pos, writable: bool) -> Stmt {
    Stmt::Bind { kind: if writable { BindKind::Let } else { BindKind::Val },
        name, ty: Some(ty), init, pos }
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

// Scratch slots are compiler-private scalars, never published as a value
// before the parser's definite-initialization proof and this independent
// lowering check agree. Every branch runs normally; no initializer is
// duplicated or speculatively evaluated.
fn constructor_block(
    body: &[Stmt], params: &Slots, slots: &Slots, initialized: &mut Slots,
    out: &mut Vec<Stmt>, pos: Pos,
) -> R<()> {
    for stmt in body {
        match stmt {
            Stmt::FieldAssign { target, field, value, pos: at }
                if matches!(&target.kind, ExprKind::SelfRef) => {
                let slot = slots.get(field).ok_or_else(|| denied(*at, "unknown constructor field"))?;
                if initialized.contains_key(field) {
                    return Err(denied(*at, "duplicate constructor initialization"));
                }
                let value = member_expr(value, params, initialized)?;
                out.push(Stmt::Assign { name: slot.clone(), value, pos: *at });
                initialized.insert(field.clone(), slot.clone());
            }
            Stmt::If { arms, otherwise, pos: at } => {
                let mut lowered = Vec::new();
                let mut joined = None;
                for (condition, branch) in arms {
                    let condition = member_expr(condition, params, initialized)?;
                    let mut state = initialized.clone();
                    let mut statements = Vec::new();
                    constructor_block(branch, params, slots, &mut state, &mut statements, *at)?;
                    if joined.as_ref().is_some_and(|prior| prior != &state) {
                        return Err(denied(*at, "constructor branch states differ"));
                    }
                    joined = Some(state);
                    lowered.push((condition, statements));
                }
                let mut state = initialized.clone();
                let other = if let Some(branch) = otherwise {
                    let mut statements = Vec::new();
                    constructor_block(branch, params, slots, &mut state, &mut statements, *at)?;
                    Some(statements)
                } else { None };
                if joined.as_ref().is_some_and(|prior| prior != &state) {
                    return Err(denied(*at, "constructor fallthrough states differ"));
                }
                *initialized = state;
                out.push(Stmt::If { arms: lowered, otherwise: other, pos: *at });
            }
            _ => return Err(denied(pos, "unsupported constructor control flow")),
        }
    }
    Ok(())
}

fn mutating_statements(m: &Method, fields: &Slots, params: &Slots) -> R<Vec<Stmt>> {
    let mut out = Vec::new();
    for (index, stmt) in m.body.iter().enumerate() {
        match stmt {
            Stmt::FieldAssign { target, field, value, pos }
                if matches!(&target.kind, ExprKind::SelfRef) => {
                let local = fields.get(field).ok_or_else(|| denied(*pos, "unknown mutable receiver field"))?;
                out.push(Stmt::Assign { name: local.clone(), value: member_expr(value, params, fields)?, pos: *pos });
            }
            Stmt::Return(None, _) if index + 1 == m.body.len() => {}
            _ => return Err(denied(m.pos, "mutable method requires closed scalar self assignments and optional final void return")),
        }
    }
    Ok(out)
}

struct MethodEntry {
    function: String,
    arity: usize,
    parameter_types: Vec<Type>,
    result: Type,
    mutable_index: Option<usize>,
}
struct LocalValue {
    fields: Vec<String>,
    writable: bool,
}
struct MainLower<'a> {
    decl: &'a StructWithTarget,
    methods: &'a BTreeMap<(String, usize), Vec<MethodEntry>>,
    type_names: &'a [(String, Pos)],
    writable_bindings: &'a [Pos],
    names: BTreeSet<String>,
    scalars: BTreeMap<String, Type>,
    values: BTreeMap<String, LocalValue>,
    next: usize,
    expansion_nodes: usize,
}

impl MainLower<'_> {
    // Resolve overloads by exact static scalar types; this is not a
    // coercion or evaluation pass. The ordinary typed compiler still
    // independently validates every emitted expression and argument.
    fn scalar_type(&self, e: &Expr) -> R<Type> {
        let invalid = || Diagnostic::new(e.pos, "E_TYPE", "expected a well-typed scalar argument");
        match &e.kind {
            ExprKind::Int(_) => Ok(Type::Int),
            ExprKind::Bool(_) => Ok(Type::Bool),
            ExprKind::Var(n) => self.scalars.get(n).copied().ok_or_else(invalid),
            ExprKind::Unary(op, a) => match (op, self.scalar_type(a)?) {
                (UnOp::Neg | UnOp::Plus | UnOp::BitNot, Type::Int) => Ok(Type::Int),
                (UnOp::Not, Type::Bool) => Ok(Type::Bool),
                _ => Err(invalid()),
            },
            ExprKind::Binary(op, a, b) => {
                let (a, b) = (self.scalar_type(a)?, self.scalar_type(b)?);
                match (op, a, b) {
                    (BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div |
                     BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor |
                     BinOp::Shl | BinOp::LShr | BinOp::AShr, Type::Int, Type::Int) => Ok(Type::Int),
                    (BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge, Type::Int, Type::Int) => Ok(Type::Bool),
                    (BinOp::Eq | BinOp::Ne, Type::Int, Type::Int) |
                    (BinOp::Eq | BinOp::Ne | BinOp::And | BinOp::Or, Type::Bool, Type::Bool) => Ok(Type::Bool),
                    _ => Err(invalid()),
                }
            }
            ExprKind::Ternary(cond, yes, no) => {
                let (c, y, n) = (self.scalar_type(cond)?, self.scalar_type(yes)?, self.scalar_type(no)?);
                if c == Type::Bool && y == n { Ok(y) } else { Err(invalid()) }
            }
            ExprKind::Call(name, _) => self.methods.values().flatten()
                .find(|entry| &entry.function == name).map(|entry| entry.result)
                .ok_or_else(invalid),
            _ => Err(invalid()),
        }
    }

    fn expr(&self, e: &Expr) -> R<Expr> {
        let pos = e.pos;
        let kind = match &e.kind {
            ExprKind::Int(n) => ExprKind::Int(*n),
            ExprKind::Bool(b) => ExprKind::Bool(*b),
            ExprKind::Var(n) if self.scalars.contains_key(n) => ExprKind::Var(n.clone()),
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
                let methods = self.methods.get(&(name.clone(), args.len())).ok_or_else(||
                    Diagnostic::new(pos, "E_ARITY", "no matching attached struct method arity"))?;
                let lowered = args.iter().map(|arg| self.expr(arg)).collect::<R<Vec<_>>>()?;
                let types = lowered.iter().map(|arg| self.scalar_type(arg)).collect::<R<Vec<_>>>()?;
                let mut candidates = methods.iter().filter(|m| m.parameter_types == types);
                let method = candidates.next().ok_or_else(||
                    Diagnostic::new(pos, "E_TYPE", "no matching attached struct method parameter types"))?;
                if candidates.next().is_some() || value.fields.len() + args.len() != method.arity {
                    return Err(denied(pos, "ambiguous or invalid flattened method signature"));
                }
                if method.mutable_index.is_some() {
                    return Err(denied(pos, "mutable struct method is a standalone void statement"));
                }
                let mut actual: Vec<_> = value.fields.iter().map(|s| variable(s, pos)).collect();
                actual.extend(lowered);
                ExprKind::Call(method.function.clone(), actual)
            }
            _ => return Err(denied(pos, "scalar struct profile does not admit this expression or value escape")),
        };
        Ok(Expr { kind, pos })
    }

    fn construct(&mut self, name: &str, args: &[Expr], pos: Pos, out: &mut Vec<Stmt>) -> R<()> {
        let writable = self.writable_bindings.contains(&pos);
        let lowered = args.iter().map(|arg| self.expr(arg)).collect::<R<Vec<_>>>()?;
        let types = lowered.iter().map(|arg| self.scalar_type(arg)).collect::<R<Vec<_>>>()?;
        let mut candidates = self.decl.constructors.iter().filter(|(_, c)|
            c.params.iter().map(|p| p.ty).eq(types.iter().copied()));
        let (_, ctor) = candidates.next().ok_or_else(|| {
            let same_arity = self.decl.constructors.iter().any(|(_, c)| c.params.len() == args.len());
            Diagnostic::new(pos, if same_arity { "E_TYPE" } else { "E_ARITY" },
                "no matching attached struct constructor signature")
        })?;
        if candidates.next().is_some() {
            return Err(denied(pos, "ambiguous attached struct constructor signature"));
        }
        charge_expansion(&mut self.expansion_nodes, body_cost(&ctor.body)
            .saturating_add(4 * (self.decl.fields.len() + args.len())), pos)?;
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
        for (i, (param, arg)) in ctor.params.iter().zip(lowered).enumerate() {
            let local = internal("arg", id, i);
            let at = arg.pos;
            out.push(binding(local.clone(), param.ty, arg, at));
            params.insert(param.name.clone(), local);
        }
        if ctor.body.iter().any(|stmt| matches!(stmt, Stmt::If { .. })) {
            let mut slots = Slots::new();
            for (i, (field, ty)) in self.decl.fields.iter().enumerate() {
                let local = internal("scratch", id, i);
                // Explicit private scratch initialization is required by
                // the ordinary scalar checker. It is not a source struct
                // default: the proof forbids reading it until assigned.
                let kind = match ty {
                    Type::Int => ExprKind::Int(Some(0)),
                    Type::Bool => ExprKind::Bool(false),
                    _ => return Err(denied(pos, "non-scalar constructor scratch")),
                };
                out.push(Stmt::Bind { kind: BindKind::Let, name: local.clone(),
                    ty: Some(*ty), init: Expr { kind, pos }, pos });
                slots.insert(field.clone(), local);
            }
            constructor_block(&ctor.body, &params, &slots, &mut initialized, out, pos)?;
            if initialized.len() != self.decl.fields.len() {
                return Err(denied(pos, "constructor fallthrough leaves a field uninitialized"));
            }
            let mut fields = Vec::new();
            for (i, (field, ty)) in self.decl.fields.iter().enumerate() {
                let scratch = initialized.get(field).ok_or_else(|| denied(pos, "missing constructor field"))?;
                let local = internal("field", id, i);
                out.push(field_binding(local.clone(), *ty, variable(scratch, pos), pos, writable));
                fields.push(local);
            }
            self.values.insert(name.to_owned(), LocalValue { fields, writable });
            return Ok(());
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
            out.push(field_binding(local.clone(), *ty, init, *at, writable));
            initialized.insert(field.clone(), local);
        }
        let fields = self.decl.fields.iter().map(|(n, _)| initialized.get(n).cloned()
            .ok_or_else(|| denied(pos, "constructor leaves uninitialized field"))).collect::<R<Vec<_>>>()?;
        self.values.insert(name.to_owned(), LocalValue { fields, writable });
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
        let writable = self.writable_bindings.contains(&pos);
        let value = self.values.get(source).ok_or_else(|| denied(pos, "copy requires a local scalar struct"))?;
        charge_expansion(&mut self.expansion_nodes, 4 * self.decl.fields.len(), pos)?;
        if value.fields.len() != self.decl.fields.len() {
            return Err(denied(pos, "invalid struct field plan"));
        }
        let id = self.next;
        self.next += 1;
        let mut fields = Vec::with_capacity(value.fields.len());
        for (i, (old, (_, ty))) in value.fields.iter().zip(&self.decl.fields).enumerate() {
            let local = internal("copy", id, i);
            out.push(field_binding(local.clone(), *ty, variable(old, pos), pos, writable));
            fields.push(local);
        }
        self.values.insert(name.to_owned(), LocalValue { fields, writable });
        Ok(())
    }

    // This bounded profile never stores a loan or passes a struct value
    // to a callback: a direct named receiver has exactly one owner. Take
    // scalar argument snapshots before writes, then execute its closed body
    // in order against only that receiver's independent storage.
    fn mutate(&mut self, base: &Expr, name: &str, args: &[Expr], pos: Pos, out: &mut Vec<Stmt>) -> R<()> {
        let ExprKind::Var(owner) = &base.kind else {
            return Err(denied(pos, "mutable receiver must be a named local struct"));
        };
        let value = self.values.get(owner).ok_or_else(|| denied(pos, "unknown struct receiver"))?;
        if !value.writable {
            return Err(Diagnostic::new(pos, "E_BORROW_MUT_REQUIRED", "mutable method needs writable local receiver authority"));
        }
        let fields: Slots = self.decl.fields.iter().zip(&value.fields)
            .map(|((name, _), local)| (name.clone(), local.clone())).collect();
        let lowered = args.iter().map(|arg| self.expr(arg)).collect::<R<Vec<_>>>()?;
        let types = lowered.iter().map(|arg| self.scalar_type(arg)).collect::<R<Vec<_>>>()?;
        let entries = self.methods.get(&(name.to_owned(), args.len())).ok_or_else(||
            Diagnostic::new(pos, "E_ARITY", "no matching attached method arity"))?;
        let mut selected = entries.iter().filter(|entry| entry.parameter_types == types);
        let entry = selected.next().ok_or_else(|| Diagnostic::new(pos, "E_TYPE", "no matching attached method signature"))?;
        if selected.next().is_some() {
            return Err(denied(pos, "ambiguous attached method"));
        }
        let index = entry.mutable_index.ok_or_else(|| denied(pos, "readonly value method is not a mutable statement"))?;
        let m = &self.decl.methods[index];
        charge_expansion(&mut self.expansion_nodes, body_cost(&m.body)
            .saturating_add(4 * args.len()), pos)?;
        let id = self.next;
        self.next += 1;
        let mut params = Slots::new();
        for (i, (p, arg)) in m.params.iter().zip(lowered).enumerate() {
            let local = internal("mut_arg", id, i);
            let at = arg.pos;
            out.push(binding(local.clone(), p.ty, arg, at));
            params.insert(p.name.clone(), local);
        }
        out.extend(mutating_statements(m, &fields, &params)?);
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
                    let inferred = self.scalar_type(&init)?;
                    if ty.is_some_and(|t| t != inferred) {
                        return Err(Diagnostic::new(*pos, "E_TYPE", "scalar binding annotation mismatch"));
                    }
                    out.push(Stmt::Bind { kind: *kind, name: name.clone(), ty: *ty, init, pos: *pos });
                    self.scalars.insert(name.clone(), inferred);
                }
                Stmt::Expr(Expr { kind: ExprKind::Call(name, args), pos })
                    if name == "stdio.println" && args.len() == 1 && !self.names.contains("stdio") => {
                    out.push(Stmt::Expr(Expr { kind: ExprKind::Call(name.clone(), vec![self.expr(&args[0])?]),
                        pos: *pos }));
                }
                Stmt::Expr(Expr { kind: ExprKind::MethodCall(base, name, args), pos }) =>
                    self.mutate(base, name, args, *pos, &mut out)?,
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
    let source_nodes = tokens.len();
    let mut parser = Parser::new(tokens);
    let decl = parser.parse_struct_with_target()?;
    let (mut program, writable_bindings) = parser.parse_struct_value_program()?;
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
        let is_mutable = decl.mutable_methods.contains(&i);
        if m.is_static || m.params.iter().any(|p| p.mutable) || (is_mutable != (m.ret == Type::Void))
            || decl.fields.len() + m.params.len() > MAX_PARAMS {
            return Err(denied(m.pos, "attached method outside scalar readonly profile"));
        }
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
        let entry = MethodEntry { function: function.clone(), arity: flat.len(),
            parameter_types: m.params.iter().map(|p| p.ty).collect(), result: m.ret,
            mutable_index: is_mutable.then_some(i) };
        let overloads = methods.entry((m.name.clone(), m.params.len())).or_insert_with(Vec::new);
        if overloads.iter().any(|prior: &MethodEntry| prior.parameter_types == entry.parameter_types) {
            return Err(denied(m.pos, "duplicate exact attached method parameter signature"));
        }
        overloads.push(entry);
        if is_mutable {
            // Validate even an unused method with the ordinary checker.
            // Its private proof function writes only local scalar scratch
            // copies, and is removed from the emitted program below.
            let mut scratch = Slots::new();
            let mut body = Vec::new();
            for (j, (name, ty)) in decl.fields.iter().enumerate() {
                let original = fields.get(name).ok_or_else(|| denied(m.pos, "missing receiver slot"))?;
                let local = internal("mut_proof", i, j);
                body.push(field_binding(local.clone(), *ty, variable(original, m.pos), m.pos, true));
                scratch.insert(name.clone(), local);
            }
            body.extend(mutating_statements(m, &scratch, &params)?);
            body.push(Stmt::Return(None, m.pos));
            lowered.push(Func { name: function, pos: m.pos, is_generator: false, is_pure: false,
                declared_effects: None, params: flat, ret: Type::Void, body });
        } else {
            let [Stmt::Return(Some(e), ret_pos)] = m.body.as_slice() else {
                return Err(denied(m.pos, "readonly method requires exactly one scalar return expression"));
            };
            lowered.push(Func { name: function, pos: m.pos, is_generator: false, is_pure: false,
                declared_effects: None, params: flat, ret: m.ret,
                body: vec![Stmt::Return(Some(member_expr(e, &params, &fields)?), *ret_pos)] });
        }
    }
    let mut lower = MainLower { decl: &decl, methods: &methods, type_names: &program.type_names, writable_bindings: &writable_bindings, names: BTreeSet::new(),
        scalars: BTreeMap::new(), values: BTreeMap::new(), next: 0, expansion_nodes: source_nodes };
    program.funcs[0].body = lower.body(&program.funcs[0].body)?;
    program.funcs.extend(lowered);
    // The proof functions allow the real typed checker to validate all
    // mutating bodies, including unused declarations. They are deliberately
    // not part of the actual emitted guest, and cannot be called from source.
    if !decl.mutable_methods.is_empty() {
        crate::check::check(&program)?;
        let proofs: BTreeSet<_> = decl.mutable_methods.iter().map(|&i| internal("method", i, 0)).collect();
        program.funcs.retain(|f| !proofs.contains(&f.name));
    }
    Ok(program)
}
