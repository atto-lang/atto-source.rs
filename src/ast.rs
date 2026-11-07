//! Untyped syntax tree produced by the parser.

use crate::diag::Pos;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Type {
    Int,
    Bool,
    Void,
    /// Nominal enum type. This ID is compiler-owned, not a user-supplied tag.
    Enum(u16),
    /// Nominal heap-allocated class reference (guest-private i32 offset).
    Class(u16),
    /// Nominal browser actor handle; never a transferable object pointer.
    Actor(u16),
}

impl Type {
    pub fn name(self) -> &'static str {
        match self {
            Type::Int => "int",
            Type::Bool => "bool",
            Type::Void => "void",
            Type::Enum(_) => "enum",
            Type::Class(_) => "class",
            Type::Actor(_) => "actor",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Program {
    pub funcs: Vec<Func>,
    /// Top-level nominal declarations. The browser backend currently lowers
    /// only free functions; unused data-only declarations need no guest code.
    pub declarations: Vec<Declaration>,
}

/// This is declaration recognition, NOT an actor or object runtime. Any
/// executable fields/methods are rejected by the parser until lowered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeclarationKind {
    Class,
    Actor,
    Struct,
    Enum,
    Module,
    Interface,
    Trait,
    Supervisor,
}

impl DeclarationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Class => "class",
            Self::Actor => "actor",
            Self::Struct => "struct",
            Self::Enum => "enum",
            Self::Module => "module",
            Self::Interface => "interface",
            Self::Trait => "trait",
            Self::Supervisor => "supervisor",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Declaration {
    pub kind: DeclarationKind,
    pub name: String,
    pub pos: Pos,
    /// Syntax-only enum variants; nominal enum values/operations are NOT
    /// lowered to browser Wasm until type checking is implemented.
    pub variants: Vec<String>,
    /// Class fields with explicit scalar defaults; no methods/actors yet.
    pub fields: Vec<ClassField>,
    /// Stage-1 stateless receive(int) -> int handler.
    pub handler: Option<Func>,
}

#[derive(Clone, Debug)]
pub struct ClassField {
    pub name: String,
    pub ty: Type,
    pub default: Expr,
    pub pos: Pos,
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
    /// A nominal enum's named variant; resolved to an i32 tag by the checker.
    EnumVariant { enum_name: String, variant: String },
    /// Field read; checked against the class's nominal schema.
    Field(Box<Expr>, String),
    Str(String),
    Var(String),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>),
    /// Class allocation requires an explicit `new ClassName(args)`.
    /// Ordinary function calls must never silently construct objects.
    NewClass(String, Vec<Expr>),
    /// Actor spawn is host-mediated; no local pseudo-actor execution.
    SpawnActor(String, Vec<Expr>),
    /// `name(args)` or the qualified builtins `stdio.print` / `stdio.println`.
    Call(String, Vec<Expr>),
}

#[derive(Clone, Debug)]
pub struct Expr {
    pub kind: ExprKind,
    pub pos: Pos,
}
