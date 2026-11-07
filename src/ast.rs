//! Untyped syntax tree produced by the parser.

use crate::diag::Pos;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Type {
    Int,
    Bool,
    Void,
}

impl Type {
    pub fn name(self) -> &'static str {
        match self {
            Type::Int => "int",
            Type::Bool => "bool",
            Type::Void => "void",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Program {
    pub funcs: Vec<Func>,
    pub decls: Vec<TypeDecl>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeclKind { Class, Actor, Enum, Struct, Interface, Trait }
impl DeclKind {
    pub fn name(self) -> &'static str {
        match self { Self::Class=>"class", Self::Actor=>"actor", Self::Enum=>"enum",
            Self::Struct=>"struct", Self::Interface=>"interface", Self::Trait=>"trait" }
    }
    pub fn tag(self) -> u8 {
        match self { Self::Class=>1, Self::Actor=>2, Self::Enum=>3,
            Self::Struct=>4, Self::Interface=>5, Self::Trait=>6 }
    }
}

#[derive(Clone, Debug)]
pub struct TypeDecl {
    pub kind: DeclKind,
    pub name: String,
    pub pos: Pos,
    pub fields: Vec<DeclField>,
    pub variants: Vec<EnumVariant>,
}
#[derive(Clone, Debug)]
pub struct EnumVariant { pub name: String, pub symbol: bool, pub pos: Pos }
#[derive(Clone, Debug)]
pub struct DeclField {
    pub name: String,
    pub ty: Type,
    pub final_slot: bool,
    pub initial: Option<DeclValue>,
    pub pos: Pos,
}
#[derive(Clone, Debug)]
pub enum DeclValue { Int(i64), Bool(bool) }

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
    Rem,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    /// `,` inside an if/while condition: short-circuit AND
    And,
    /// `|` inside an if/while condition: short-circuit OR
    Or,
}

impl BinOp {
    pub fn symbol(self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Rem => "%",
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
}

#[derive(Clone, Debug)]
pub struct Expr {
    pub kind: ExprKind,
    pub pos: Pos,
}
