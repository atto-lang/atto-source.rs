//! Syntax tree of the general ("v2") pipeline: the async, pattern-matching and
//! actor-composition surface that the legacy narrow-profile compiler rejects.

use crate::diag::Pos;

/// A type as written in source. Resolved by the checker.
#[derive(Clone, Debug, PartialEq)]
pub enum TyEx {
    /// `Name` or `Name<A, B>`
    Name(String, Vec<TyEx>, Pos),
    /// `(A, B)`
    Tuple(Vec<TyEx>, Pos),
}

impl TyEx {
    pub fn pos(&self) -> Pos {
        match self {
            TyEx::Name(_, _, p) | TyEx::Tuple(_, p) => *p,
        }
    }
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
    /// `&&`: short-circuit AND
    And,
    /// `||`: short-circuit OR
    Or,
    /// `^^`: boolean XOR (both operands evaluated)
    Xor,
    /// `,` inside an if/while condition: eager AND
    CondAnd,
    /// `|` inside an if/while condition: eager OR
    CondOr,
    /// `a | b` / `a pipe b` between actor references: link a pipeline
    Pipe,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadMode {
    /// `readch ch`: suspend until an item arrives
    Wait,
    /// `try readch ch`: immediate `Option<T>`
    Try,
    /// `nb readch ch`: register a waiter, `Future<Option<T>>`
    Nb,
}

#[derive(Clone, Debug)]
pub struct Arg {
    pub name: Option<String>,
    pub value: Expr,
}

#[derive(Clone, Debug)]
pub struct LambdaParam {
    pub name: String,
    pub ty: Option<TyEx>,
    pub pos: Pos,
}

#[derive(Clone, Debug)]
pub enum Pat {
    /// `_` or `default`
    Wild,
    Int(i64),
    Bool(bool),
    Str(String),
    /// lower-case name: binds the subject
    Bind(String),
    /// `None`, `Some(x)`, `Ok(x)`, `Err(x)`; the payload binding may be `_`
    Variant(String, Option<String>),
    /// `is T`
    Is(TyEx),
}

#[derive(Clone, Debug)]
pub enum ArmBody {
    Block(Vec<Stmt>),
    Expr(Expr),
}

#[derive(Clone, Debug)]
pub struct Arm {
    pub pat: Pat,
    pub guard: Option<Expr>,
    pub body: ArmBody,
    pub pos: Pos,
}

#[derive(Clone, Debug)]
pub enum Ex {
    /// `None` is 2^63, legal only under unary minus.
    Int(Option<u64>),
    Bool(bool),
    Str(String),
    Var(String),
    SelfRef,
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>),
    /// `name(args)` or `name<T>(args)`
    Call { name: String, targs: Vec<TyEx>, args: Vec<Arg> },
    /// `base.name(args)`
    Method { base: Box<Expr>, name: String, args: Vec<Arg> },
    /// `base.name`
    Member(Box<Expr>, String),
    /// `Type<T>.name(args)`, e.g. `Channel<int>.bounded(2)`
    TypeStatic { ty: TyEx, name: String, args: Vec<Arg> },
    Tuple(Vec<Expr>),
    List(Vec<Expr>),
    Lambda { params: Vec<LambdaParam>, ret: Option<TyEx>, body: Vec<Stmt> },
    Await(Box<Expr>),
    /// `task { ... }`: start a task, yield its `Future<T>`
    Task(Vec<Stmt>),
    Spawn(String, Vec<Arg>),
    ReadCh(ReadMode, Box<Expr>),
    Match(Box<Expr>, Vec<Arm>),
    /// `value is T`
    Is(Box<Expr>, TyEx),
}

#[derive(Clone, Debug)]
pub struct Expr {
    pub kind: Ex,
    pub pos: Pos,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindKind {
    Val,
    Const,
    Let,
}

#[derive(Clone, Debug)]
pub enum BindTarget {
    Name(String),
    /// `val (a, b) = tuple`
    Tuple(Vec<String>),
}

#[derive(Clone, Debug)]
pub enum Stmt {
    Bind { kind: BindKind, target: BindTarget, ty: Option<TyEx>, init: Expr, pos: Pos },
    Assign { name: String, value: Expr, pos: Pos },
    FieldAssign { target: Expr, field: String, value: Expr, pos: Pos },
    Expr(Expr),
    Return(Option<Expr>, Pos),
    If { arms: Vec<(Expr, Vec<Stmt>)>, otherwise: Option<Vec<Stmt>>, pos: Pos },
    While { cond: Expr, body: Vec<Stmt>, pos: Pos },
    Loop { body: Vec<Stmt>, pos: Pos },
    For { init: Option<Box<Stmt>>, cond: Option<Expr>, update: Option<Box<Stmt>>, body: Vec<Stmt>, pos: Pos },
    /// `for x in source` / `for await x in source`
    ForIn { name: String, source: Expr, body: Vec<Stmt>, is_await: bool, pos: Pos },
    Break(Pos),
    Continue(Pos),
    Block(Vec<Stmt>),
    Yield(Expr, Pos),
    YieldFrom(Expr, Pos),
    /// `writech ch, value`
    WriteCh { ch: Expr, value: Expr, pos: Pos },
    /// `do task { ... }`: detached task, no Future
    DoTask(Vec<Stmt>, Pos),
    /// `using resource [as name] { ... }`
    Using { resource: Expr, name: Option<String>, body: Vec<Stmt>, pos: Pos },
    /// `with scope { ... }`
    With { scope: Expr, body: Vec<Stmt>, pos: Pos },
}

#[derive(Clone, Debug)]
pub struct Param {
    pub name: String,
    pub ty: TyEx,
    pub mutable: bool,
    pub pos: Pos,
}

#[derive(Clone, Debug)]
pub struct Func {
    pub name: String,
    pub pos: Pos,
    pub suspends: bool,
    pub is_gen: bool,
    pub params: Vec<Param>,
    /// `None` is `void`
    pub ret: Option<TyEx>,
    pub body: Vec<Stmt>,
}

#[derive(Clone, Debug)]
pub struct Field {
    pub name: String,
    pub pos: Pos,
    pub ty: Option<TyEx>,
    pub mutable: bool,
    pub init: Expr,
}

#[derive(Clone, Debug)]
pub struct Receive {
    pub pos: Pos,
    pub mail: String,
    pub payload: TyEx,
    pub body: Vec<Stmt>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjKind {
    Actor,
    Supervisor,
}

/// `define actor|supervisor Name as ... end`
#[derive(Clone, Debug)]
pub struct Object {
    pub kind: ObjKind,
    pub name: String,
    pub pos: Pos,
    pub fields: Vec<Field>,
    pub receive: Option<Receive>,
    pub methods: Vec<Func>,
}

#[derive(Clone, Debug, Default)]
pub struct Program {
    pub funcs: Vec<Func>,
    pub objects: Vec<Object>,
    /// `use a.b.c;`
    pub uses: Vec<(Vec<String>, Pos)>,
}
