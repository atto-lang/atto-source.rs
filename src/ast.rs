//! Untyped syntax tree produced by the parser.

use crate::diag::Pos;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Type {
    Int,
    Bool,
    Void,
    Enum(u16), // nominal per-declaration enum identity
    Class(u16), // nominal per-declaration class identity; no object ABI yet
}

impl Type {
    pub fn name(self) -> &'static str {
        match self {
            Type::Int => "int",
            Type::Bool => "bool",
            Type::Void => "void",
            Type::Enum(_) => "enum",
            Type::Class(_) => "class",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Program {
    pub funcs: Vec<Func>,
    pub decls: Vec<Declaration>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeclKind { Class, Actor, Enum, Struct, Interface, Trait, Module }

#[derive(Clone, Debug)]
pub struct Declaration {
    pub name: String,
    pub pos: Pos,
    pub kind: DeclKind,
    pub variants: Vec<String>,
    /// Class instance slot declarations, retained for M2 layout/type checking.
    /// No field is emitted until actor- and object-lifetime rules are implemented.
    pub fields: Vec<ClassField>,
}

#[derive(Clone, Debug)]
pub struct ClassField {
    pub name: String,
    pub pos: Pos,
    pub ty: Type,
    pub initializer: Expr,
    pub final_slot: bool,
    pub require_const: bool,
}


#[derive(Clone, Debug)]
pub struct Func {
    pub name: String,
    pub pos: Pos,
    pub params: Vec<Param>,
    pub ret: Type,
    pub body: Vec<Stmt>,
}

#[derive(Clone, Debug)]
pub struct Param {
    pub name: String,
    pub ty: Type,
    pub mutable: bool,
    pub pos: Pos,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindKind {
    /// `val`
    Val,
    /// `const`: initializer must be a compile-time constant
    Const,
    /// `let`
    Let,
}

#[derive(Clone, Debug)]
pub enum Stmt {
    Bind {
        kind: BindKind,
        name: String,
        ty: Option<Type>,
        init: Expr,
        pos: Pos,
    },
    Assign {
        name: String,
        value: Expr,
        pos: Pos,
    },
    Expr(Expr),
    Return(Option<Expr>, Pos),
    If {
        arms: Vec<(Expr, Vec<Stmt>)>,
        otherwise: Option<Vec<Stmt>>,
        pos: Pos,
    },
    While {
        cond: Expr,
        body: Vec<Stmt>,
        pos: Pos,
    },
    Loop {
        body: Vec<Stmt>,
        pos: Pos,
    },
    For {
        init: Option<Box<Stmt>>,
        cond: Option<Expr>,
        update: Option<Box<Stmt>>,
        body: Vec<Stmt>,
        pos: Pos,
    },
    Break(Pos),
    Continue(Pos),
    Block(Vec<Stmt>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Plus,
    Not,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    /// `,` inside an if/while condition: eager, left-to-right AND
    And,
    /// `|` inside an if/while condition: eager, left-to-right OR
    Or,
}

impl BinOp {
    pub fn symbol(self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Eq => "eq",
            BinOp::Ne => "neq",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::And => ",",
            BinOp::Or => "|",
        }
    }
}

#[derive(Clone, Debug)]
pub enum ExprKind {
    /// Integer literal. `None` is 2^63, only legal as the operand of unary minus.
    Int(Option<u64>),
    Bool(bool),
    Str(String),
    Var(String),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>),
    /// `name(args)` or the qualified builtins `stdio.print` / `stdio.println`.
    Call(String, Vec<Expr>),
    /// Parsed constructor call, not lowered until class arenas/ownership exist.
    New(String, Vec<Expr>),
}

#[derive(Clone, Debug)]
pub struct Expr {
    pub kind: ExprKind,
    pub pos: Pos,
}
