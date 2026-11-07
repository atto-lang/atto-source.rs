//! Name resolution and type checking: AST -> typed IR.
//!
//! Rules follow the reference compilers and the spec:
//! * `int` is a signed 64-bit integer (spec 17, LLVM `i64`), arithmetic is
//!   checked and overflow panics (spec 17 TARGET (decided); LLVM traps).
//! * conditions must be `bool`; there is no truthiness (spec 17).
//! * `val`/`const` bindings and parameters are immutable; `let` and
//!   `type mut name` parameters are reassignable (grammar.ebnf, Java).
//! * top-level overload identity is name + arity (Java reference).
//! * a non-void function must return on every path (Java reference).
//! * `int / int` is rejected: its result type is undecided (spec S-25; Java
//!   yields `float`, LLVM rejects).

use std::collections::BTreeMap;

use crate::ast::*;
use crate::diag::{Diagnostic, Pos};

pub const MAX_LOCALS: usize = 1024;
pub const MAX_STRING_DATA: usize = 32 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Builtin {
    And,
    Or,
    Xor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Import {
    PrintI64,
    PrintBool,
    PrintStr,
    PrintNewline,
}

impl Import {
    pub const ALL: [Import; 4] = [Import::PrintI64, Import::PrintBool, Import::PrintStr, Import::PrintNewline];
    pub fn name(self) -> &'static str {
        match self {
            Import::PrintI64 => "print_i64",
            Import::PrintBool => "print_bool",
            Import::PrintStr => "print_str",
            Import::PrintNewline => "print_newline",
        }
    }
}

#[derive(Clone, Debug)]
pub enum TExpr {
    I64(i64),
    Bool(bool),
    Local(u32),
    Neg(Box<TE>),
    Not(Box<TE>),
    /// operand type is the type of `l`
    Bin(BinOp, Box<TE>, Box<TE>),
    Ternary(Box<TE>, Box<TE>, Box<TE>),
    Call(u32, Vec<TE>),
    Builtin(Builtin, Box<TE>, Box<TE>),
    /// `stdio.print` / `stdio.println` of an int or bool value
    Print {
        value: Box<TE>,
        newline: bool,
    },
    /// `stdio.print` / `stdio.println` of a string literal at data offset
    PrintStr {
        offset: u32,
        len: u32,
        newline: bool,
    },
}

#[derive(Clone, Debug)]
pub struct TE {
    pub e: TExpr,
    pub ty: Type,
}

#[derive(Clone, Debug)]
pub enum TStmt {
    Set(u32, TE),
    Expr(TE),
    Return(Option<TE>),
    If(Vec<(TE, Vec<TStmt>)>, Option<Vec<TStmt>>),
    /// `while`, `loop` and C-style `for`: `cond` absent means unconditional;
    /// `update` runs after the body and after `continue`.
    Loop {
        cond: Option<TE>,
        body: Vec<TStmt>,
        update: Vec<TStmt>,
    },
    Break,
    Continue,
    Block(Vec<TStmt>),
}

#[derive(Clone, Debug)]
pub struct TFunc {
    pub name: String,
    pub params: Vec<Type>,
    pub ret: Type,
    /// declared locals after the parameters
    pub locals: Vec<Type>,
    pub body: Vec<TStmt>,
}

#[derive(Clone, Debug)]
pub struct TProgram {
    pub funcs: Vec<TFunc>,
    pub main: u32,
    pub imports: Vec<Import>,
    pub data: Vec<u8>,
    pub decls: Vec<TypeDecl>,
}

struct Local {
    name: String,
    slot: u32,
    ty: Type,
    mutable: bool,
}

struct FnCx<'a> {
    sigs: &'a BTreeMap<(String, usize), (u32, Vec<Type>, Type)>,
    names: &'a BTreeMap<String, Vec<usize>>,
    ret: Type,
    scopes: Vec<Vec<Local>>,
    locals: Vec<Type>,
    nparams: u32,
    loops: u32,
    imports: &'a mut Vec<Import>,
    data: &'a mut Vec<u8>,
}

type R<T> = Result<T, Diagnostic>;

fn type_err(pos: Pos, msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new(pos, "E_TYPE", msg)
}

pub fn check(p: &Program) -> R<TProgram> {
    let mut declared = BTreeMap::new();
    for d in &p.decls {
        if let Some(previous) = declared.insert(d.name.clone(), d.kind) {
            return Err(Diagnostic::new(d.pos, "E_DUPLICATE",
                format!("type '{}' already declared as {}", d.name, previous.name())));
        }
        if matches!(d.name.as_str(), "main" | "stdio" | "And" | "Or" | "Xor") {
            return Err(Diagnostic::new(d.pos, "E_DUPLICATE", "declaration conflicts with reserved runtime name"));
        }
    }
    let mut sigs = BTreeMap::new();
    let mut names: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, f) in p.funcs.iter().enumerate() {
        if declared.contains_key(&f.name) {
            return Err(Diagnostic::new(f.pos, "E_DUPLICATE", format!("function conflicts with type {}", f.name)));
        }
        if matches!(f.name.as_str(), "And" | "Or" | "Xor" | "stdio") {
            return Err(Diagnostic::new(
                f.pos,
                "E_DUPLICATE",
                format!("'{}' is a builtin and cannot be redefined", f.name),
            ));
        }
        let key = (f.name.clone(), f.params.len());
        if sigs.contains_key(&key) {
            return Err(Diagnostic::new(
                f.pos,
                "E_DUPLICATE",
                format!(
                    "function '{}' already has arity {}; overload identity is name + arity",
                    f.name,
                    f.params.len()
                ),
            ));
        }
        sigs.insert(key, (i as u32, f.params.iter().map(|p| p.ty).collect(), f.ret));
        names.entry(f.name.clone()).or_default().push(f.params.len());
    }
    let main = match sigs.get(&("main".to_string(), 0)) {
        Some((i, _, ret)) => {
            if !matches!(ret, Type::Int | Type::Void) {
                return Err(Diagnostic::new(
                    p.funcs[*i as usize].pos,
                    "E_ENTRY",
                    "main() must return int or void",
                ));
            }
            *i
        }
        None => {
            let pos = p
                .funcs
                .iter()
                .find(|f| f.name == "main")
                .map(|f| f.pos)
                .unwrap_or(Pos { line: 1, col: 1 });
            return Err(Diagnostic::new(
                pos,
                "E_ENTRY",
                "executable source requires a zero-argument main() entrypoint",
            ));
        }
    };
    let mut imports = Vec::new();
    let mut data = Vec::new();
    let mut funcs = Vec::new();
    for f in &p.funcs {
        let mut cx = FnCx {
            sigs: &sigs,
            names: &names,
            ret: f.ret,
            scopes: vec![Vec::new()],
            locals: Vec::new(),
            nparams: f.params.len() as u32,
            loops: 0,
            imports: &mut imports,
            data: &mut data,
        };
        for (i, prm) in f.params.iter().enumerate() {
            cx.scopes[0].push(Local {
                name: prm.name.clone(),
                slot: i as u32,
                ty: prm.ty,
                mutable: prm.mutable,
            });
        }
        cx.scopes.push(Vec::new());
        let body = cx.stmts(&f.body)?;
        if f.ret != Type::Void && !returns(&body) {
            return Err(Diagnostic::new(
                f.pos,
                "E_RETURN",
                format!("non-void fnc '{}' must explicitly return on every path", f.name),
            ));
        }
        funcs.push(TFunc {
            name: f.name.clone(),
            params: f.params.iter().map(|p| p.ty).collect(),
            ret: f.ret,
            locals: cx.locals,
            body,
        });
    }
    imports.sort();
    imports.dedup();
    Ok(TProgram {
        funcs,
        main,
        imports,
        data,
        decls: p.decls.clone(),
    })
}

/// Definite-return analysis matching the Java reference: an `if` returns
/// only with an `else` whose every arm returns; an unconditional `loop`
/// without a `break` never falls through; `while true` is not special.
fn returns(body: &[TStmt]) -> bool {
    body.iter().any(|s| match s {
        TStmt::Return(_) => true,
        TStmt::If(arms, Some(other)) => arms.iter().all(|(_, b)| returns(b)) && returns(other),
        TStmt::Loop { cond: None, body, .. } => !breaks(body),
        TStmt::Block(b) => returns(b),
        _ => false,
    })
}

/// Whether a loop body contains a `break` that targets this loop.
fn breaks(body: &[TStmt]) -> bool {
    body.iter().any(|s| match s {
        TStmt::Break => true,
        TStmt::If(arms, other) => arms.iter().any(|(_, b)| breaks(b)) || other.as_ref().is_some_and(|b| breaks(b)),
        TStmt::Block(b) => breaks(b),
        _ => false,
    })
}

impl FnCx<'_> {
    fn lookup(&self, name: &str) -> Option<&Local> {
        self.scopes.iter().rev().flat_map(|s| s.iter().rev()).find(|l| l.name == name)
    }

    fn declare(&mut self, name: &str, ty: Type, mutable: bool, pos: Pos) -> R<u32> {
        if self.scopes.last().unwrap().iter().any(|l| l.name == name) {
            return Err(Diagnostic::new(pos, "E_DUPLICATE", format!("duplicate binding '{name}'")));
        }
        if self.locals.len() >= MAX_LOCALS {
            return Err(Diagnostic::new(
                pos,
                "E_SOURCE_LIMIT",
                format!("more than {MAX_LOCALS} locals in one function"),
            ));
        }
        let slot = self.nparams + self.locals.len() as u32;
        self.locals.push(ty);
        self.scopes.last_mut().unwrap().push(Local {
            name: name.to_string(),
            slot,
            ty,
            mutable,
        });
        Ok(slot)
    }

    fn scoped(&mut self, body: &[Stmt]) -> R<Vec<TStmt>> {
        self.scopes.push(Vec::new());
        let r = self.stmts(body);
        self.scopes.pop();
        r
    }

    fn stmts(&mut self, body: &[Stmt]) -> R<Vec<TStmt>> {
        body.iter().map(|s| self.stmt(s)).collect()
    }

    fn cond(&mut self, e: &Expr, what: &str) -> R<TE> {
        let c = self.expr(e)?;
        if c.ty != Type::Bool {
            return Err(type_err(
                e.pos,
                format!("{what} condition must have type bool, found {}", c.ty.name()),
            ));
        }
        Ok(c)
    }

    fn stmt(&mut self, s: &Stmt) -> R<TStmt> {
        Ok(match s {
            Stmt::Bind { kind, name, ty, init, pos } => {
                let v = self.value(init)?;
                if let Some(t) = ty {
                    if *t != v.ty {
                        return Err(type_err(
                            init.pos,
                            format!(
                                "binding '{name}' has type {} but its initializer has type {}",
                                t.name(),
                                v.ty.name()
                            ),
                        ));
                    }
                }
                let slot = self.declare(name, v.ty, *kind == BindKind::Let, *pos)?;
                TStmt::Set(slot, v)
            }
            Stmt::Assign { name, value, pos } => {
                let (slot, ty, mutable) = match self.lookup(name) {
                    Some(l) => (l.slot, l.ty, l.mutable),
                    None => return Err(Diagnostic::new(*pos, "E_UNBOUND", format!("unknown name '{name}'"))),
                };
                if !mutable {
                    return Err(Diagnostic::new(
                        *pos,
                        "E_IMMUTABLE",
                        format!("cannot reassign val binding '{name}'"),
                    ));
                }
                let v = self.value(value)?;
                if v.ty != ty {
                    return Err(type_err(
                        value.pos,
                        format!("cannot assign {} to '{name}' of type {}", v.ty.name(), ty.name()),
                    ));
                }
                TStmt::Set(slot, v)
            }
            Stmt::Expr(e) => TStmt::Expr(self.expr(e)?),
            Stmt::Return(v, pos) => match (v, self.ret) {
                (None, Type::Void) => TStmt::Return(None),
                (None, t) => {
                    return Err(Diagnostic::new(
                        *pos,
                        "E_RETURN",
                        format!("return needs a value of type {}", t.name()),
                    ))
                }
                (Some(e), Type::Void) => return Err(type_err(e.pos, "a void fnc cannot return a value")),
                (Some(e), t) => {
                    let v = self.value(e)?;
                    if v.ty != t {
                        return Err(type_err(
                            e.pos,
                            format!("return value has type {} but expected {}", v.ty.name(), t.name()),
                        ));
                    }
                    TStmt::Return(Some(v))
                }
            },
            Stmt::If { arms, otherwise, .. } => {
                let mut out = Vec::new();
                for (c, b) in arms {
                    let c = self.cond(c, "if")?;
                    out.push((c, self.scoped(b)?));
                }
                let other = match otherwise {
                    Some(b) => Some(self.scoped(b)?),
                    None => None,
                };
                TStmt::If(out, other)
            }
            Stmt::While { cond, body, .. } => {
                let c = self.cond(cond, "while")?;
                self.loops += 1;
                let body = self.scoped(body);
                self.loops -= 1;
                TStmt::Loop {
                    cond: Some(c),
                    body: body?,
                    update: Vec::new(),
                }
            }
            Stmt::Loop { body, .. } => {
                self.loops += 1;
                let body = self.scoped(body);
                self.loops -= 1;
                TStmt::Loop {
                    cond: None,
                    body: body?,
                    update: Vec::new(),
                }
            }
            Stmt::For {
                init, cond, update, body, ..
            } => {
                self.scopes.push(Vec::new());
                let r = (|| {
                    let mut out = Vec::new();
                    if let Some(i) = init {
                        out.push(self.stmt(i)?);
                    }
                    let c = match cond {
                        Some(c) => Some(self.cond(c, "for")?),
                        None => None,
                    };
                    self.loops += 1;
                    let b = self.scoped(body);
                    self.loops -= 1;
                    let u = match update {
                        Some(u) => vec![self.stmt(u)?],
                        None => Vec::new(),
                    };
                    out.push(TStmt::Loop {
                        cond: c,
                        body: b?,
                        update: u,
                    });
                    Ok(TStmt::Block(out))
                })();
                self.scopes.pop();
                r?
            }
            Stmt::Break(pos) | Stmt::Continue(pos) => {
                if self.loops == 0 {
                    let w = if matches!(s, Stmt::Break(_)) { "break" } else { "continue" };
                    return Err(Diagnostic::new(
                        *pos,
                        "E_LOOP",
                        format!("'{w}' may only appear inside loop, while or for"),
                    ));
                }
                if matches!(s, Stmt::Break(_)) {
                    TStmt::Break
                } else {
                    TStmt::Continue
                }
            }
            Stmt::Block(b) => TStmt::Block(self.scoped(b)?),
        })
    }

    /// An expression used as a value (not void).
    fn value(&mut self, e: &Expr) -> R<TE> {
        let v = self.expr(e)?;
        if v.ty == Type::Void {
            return Err(type_err(e.pos, "a void call has no value"));
        }
        Ok(v)
    }

    fn expr(&mut self, e: &Expr) -> R<TE> {
        let pos = e.pos;
        Ok(match &e.kind {
            ExprKind::Int(Some(v)) => TE {
                e: TExpr::I64(*v as i64),
                ty: Type::Int,
            },
            ExprKind::Int(None) => {
                return Err(Diagnostic::new(
                    pos,
                    "E_INTEGER_RANGE",
                    "integer literal does not fit in a signed 64-bit int",
                ))
            }
            ExprKind::Bool(b) => TE {
                e: TExpr::Bool(*b),
                ty: Type::Bool,
            },
            ExprKind::Str(_) => {
                return Err(Diagnostic::new(
                    pos,
                    "E_UNSUPPORTED_TYPE",
                    "String values are not supported by the browser compiler yet (string literals may only be printed)",
                ))
            }
            ExprKind::Var(name) => match self.lookup(name) {
                Some(l) => TE {
                    e: TExpr::Local(l.slot),
                    ty: l.ty,
                },
                None if self.names.contains_key(name) => {
                    return Err(Diagnostic::new(
                        pos,
                        "E_UNSUPPORTED",
                        "functions as values are not supported by the browser compiler yet",
                    ))
                }
                None => return Err(Diagnostic::new(pos, "E_UNBOUND", format!("unknown name '{name}'"))),
            },
            ExprKind::Unary(UnOp::Neg, inner) if matches!(inner.kind, ExprKind::Int(None)) => TE {
                e: TExpr::I64(i64::MIN),
                ty: Type::Int,
            },
            ExprKind::Unary(op, inner) => {
                let v = self.value(inner)?;
                match op {
                    UnOp::Neg | UnOp::Plus => {
                        if v.ty != Type::Int {
                            return Err(type_err(
                                pos,
                                format!(
                                    "unary '{}' needs an int operand, found {}",
                                    if *op == UnOp::Neg { "-" } else { "+" },
                                    v.ty.name()
                                ),
                            ));
                        }
                        if *op == UnOp::Plus {
                            v
                        } else {
                            TE {
                                e: TExpr::Neg(Box::new(v)),
                                ty: Type::Int,
                            }
                        }
                    }
                    UnOp::Not => {
                        if v.ty != Type::Bool {
                            return Err(type_err(pos, format!("'!' needs a bool operand, found {}", v.ty.name())));
                        }
                        TE {
                            e: TExpr::Not(Box::new(v)),
                            ty: Type::Bool,
                        }
                    }
                }
            }
            ExprKind::Binary(op, l, r) => {
                let a = self.value(l)?;
                let b = self.value(r)?;
                let ty = match op {
                    BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Rem => {
                        if a.ty != Type::Int || b.ty != Type::Int {
                            return Err(type_err(
                                pos,
                                format!(
                                    "operator '{}' needs int operands, found {} and {}",
                                    op.symbol(),
                                    a.ty.name(),
                                    b.ty.name()
                                ),
                            ));
                        }
                        Type::Int
                    }
                    BinOp::Div => {
                        return Err(Diagnostic::new(
                            pos,
                            "E_UNSUPPORTED",
                            "'/' on int is not admitted: whether int / int yields float or int is undecided (spec S-25)",
                        ))
                    }
                    BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                        if a.ty != Type::Int || b.ty != Type::Int {
                            return Err(type_err(
                                pos,
                                format!(
                                    "operator '{}' needs int operands, found {} and {}",
                                    op.symbol(),
                                    a.ty.name(),
                                    b.ty.name()
                                ),
                            ));
                        }
                        Type::Bool
                    }
                    BinOp::Eq | BinOp::Ne => {
                        if a.ty != b.ty {
                            return Err(type_err(
                                pos,
                                format!(
                                    "'{}' operands must have the same type, found {} and {}",
                                    op.symbol(),
                                    a.ty.name(),
                                    b.ty.name()
                                ),
                            ));
                        }
                        Type::Bool
                    }
                    BinOp::And | BinOp::Or => {
                        if a.ty != Type::Bool || b.ty != Type::Bool {
                            return Err(type_err(pos, format!("condition operator '{}' needs bool operands", op.symbol())));
                        }
                        Type::Bool
                    }
                };
                TE {
                    e: TExpr::Bin(*op, Box::new(a), Box::new(b)),
                    ty,
                }
            }
            ExprKind::Ternary(c, a, b) => {
                let c = self.cond(c, "ternary")?;
                let a = self.value(a)?;
                let b = self.value(b)?;
                if a.ty != b.ty {
                    return Err(type_err(
                        pos,
                        format!(
                            "ternary branches must have the same type, found {} and {}",
                            a.ty.name(),
                            b.ty.name()
                        ),
                    ));
                }
                let ty = a.ty;
                TE {
                    e: TExpr::Ternary(Box::new(c), Box::new(a), Box::new(b)),
                    ty,
                }
            }
            ExprKind::Call(name, args) => self.call(name, args, pos)?,
        })
    }

    fn call(&mut self, name: &str, args: &[Expr], pos: Pos) -> R<TE> {
        if let Some(b) = match name {
            "And" => Some(Builtin::And),
            "Or" => Some(Builtin::Or),
            "Xor" => Some(Builtin::Xor),
            _ => None,
        } {
            if self.lookup(name).is_none() {
                if args.len() != 2 {
                    return Err(Diagnostic::new(
                        pos,
                        "E_ARITY",
                        format!("{name} takes 2 arguments, found {}", args.len()),
                    ));
                }
                let a = self.value(&args[0])?;
                let c = self.value(&args[1])?;
                if a.ty != Type::Bool || c.ty != Type::Bool {
                    return Err(type_err(pos, format!("{name} needs bool arguments")));
                }
                return Ok(TE {
                    e: TExpr::Builtin(b, Box::new(a), Box::new(c)),
                    ty: Type::Bool,
                });
            }
        }
        if name == "stdio.print" || name == "stdio.println" {
            let newline = name == "stdio.println";
            if args.len() != 1 {
                return Err(Diagnostic::new(
                    pos,
                    "E_ARITY",
                    format!("{name} takes 1 argument, found {}", args.len()),
                ));
            }
            if newline {
                self.imports.push(Import::PrintNewline);
            }
            if let ExprKind::Str(s) = &args[0].kind {
                if self.data.len() + s.len() > MAX_STRING_DATA {
                    return Err(Diagnostic::new(pos, "E_SOURCE_LIMIT", "too much string literal data"));
                }
                let offset = self.data.len() as u32;
                self.data.extend_from_slice(s.as_bytes());
                self.imports.push(Import::PrintStr);
                return Ok(TE {
                    e: TExpr::PrintStr {
                        offset,
                        len: s.len() as u32,
                        newline,
                    },
                    ty: Type::Void,
                });
            }
            let v = self.value(&args[0])?;
            self.imports
                .push(if v.ty == Type::Int { Import::PrintI64 } else { Import::PrintBool });
            return Ok(TE {
                e: TExpr::Print {
                    value: Box::new(v),
                    newline,
                },
                ty: Type::Void,
            });
        }
        if self.lookup(name).is_some() {
            return Err(Diagnostic::new(
                pos,
                "E_UNSUPPORTED",
                format!("'{name}' is a local value, not a function"),
            ));
        }
        let (idx, params, ret) = match self.sigs.get(&(name.to_string(), args.len())) {
            Some(s) => s.clone(),
            None => {
                return Err(match self.names.get(name) {
                    Some(arities) => Diagnostic::new(
                        pos,
                        "E_ARITY",
                        format!(
                            "call arity mismatch: '{name}' takes {} argument(s), found {}",
                            arities.iter().map(|a| a.to_string()).collect::<Vec<_>>().join(" or "),
                            args.len()
                        ),
                    ),
                    None => Diagnostic::new(pos, "E_UNBOUND", format!("unknown name '{name}'")),
                })
            }
        };
        let mut targs = Vec::new();
        for (i, (a, want)) in args.iter().zip(params.iter()).enumerate() {
            let v = self.value(a)?;
            if v.ty != *want {
                return Err(type_err(
                    a.pos,
                    format!(
                        "argument {} of '{name}' has type {} but expected {}",
                        i + 1,
                        v.ty.name(),
                        want.name()
                    ),
                ));
            }
            targs.push(v);
        }
        Ok(TE {
            e: TExpr::Call(idx, targs),
            ty: ret,
        })
    }
}
