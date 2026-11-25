//! Uniform-value IR of the v2 pipeline. Every value is one `i64`: integers
//! directly, booleans as 0/1, strings as `ptr << 32 | len`, everything else
//! as a pointer into the module's bounded bump heap.

/// Wasm globals of a v2 module.
pub const G_PANIC: u32 = 0;
pub const G_HEAP: u32 = 1;
/// Coroutine state: 0 running, 1 unwinding to the scheduler, 2 rewinding.
pub const G_STATE: u32 = 2;
/// Top of the current task's saved-frame stack.
pub const G_FSP: u32 = 3;
/// End of the current task's saved-frame stack.
pub const G_FEND: u32 = 4;

pub const STATE_RUN: i32 = 0;
pub const STATE_UNWIND: i32 = 1;
pub const STATE_REWIND: i32 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    /// wrapping i64 arithmetic for runtime address math
    AddRaw,
    SubRaw,
    MulRaw,
    Eq,
    Ne,
    LtS,
    LeS,
    GtS,
    GeS,
    And,
    Or,
    Xor,
    Shl,
    ShrU,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Helper {
    /// checked signed 64-bit arithmetic; overflow sets panic_code 1 and traps
    Add,
    Sub,
    Mul,
    Neg,
    /// bump allocation of `n` zeroed bytes
    Alloc,
}

#[derive(Clone, Debug)]
pub enum Ir {
    Const(i64),
    Local(u32),
    Global(u32),
    /// `i64.load` at the address held in the operand
    Load(Box<Ir>),
    /// `i64.load8_u`
    Load8(Box<Ir>),
    Bin(Op, Box<Ir>, Box<Ir>),
    /// logical not of a 0/1 value
    Not(Box<Ir>),
    /// direct call of IR function `id`
    Call(u32, Vec<Ir>),
    /// call through a closure: the first operand is the closure pointer,
    /// whose first word is the function-table slot; it is also passed as
    /// the callee's environment argument.
    CallClosure(Box<Ir>, Vec<Ir>),
    Helper(Helper, Vec<Ir>),
    If(Box<Ir>, Box<Ir>, Box<Ir>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Print {
    I64,
    Bool,
    /// packed string
    Str,
    Newline,
}

#[derive(Clone, Debug)]
pub enum IrStmt {
    Set(u32, Ir),
    /// `i64.store` of `value` at address `addr`
    Store(Ir, Ir),
    Store8(Ir, Ir),
    GlobalSet(u32, Ir),
    Expr(Ir),
    If(Ir, Vec<IrStmt>, Vec<IrStmt>),
    /// Infinite loop left by `Break`; `Continue` runs `update` first.
    Loop { body: Vec<IrStmt>, update: Vec<IrStmt> },
    Break,
    Continue,
    Return(Ir),
    /// Call of a suspending function. Arguments are constants or locals so
    /// that re-evaluating them while rewinding has no effect.
    SCall(Option<u32>, u32, Vec<Ir>),
    /// Yield to the scheduler; execution continues here when resumed.
    Suspend,
    Print(Print, Option<Ir>),
    Panic(i32),
}

#[derive(Clone, Debug)]
pub struct IrFn {
    pub name: String,
    pub params: u32,
    /// total locals including parameters
    pub locals: u32,
    pub body: Vec<IrStmt>,
    /// compiled as a resumable state machine
    pub suspends: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Module {
    pub fns: Vec<IrFn>,
    /// IR function exported as `main: () -> i64`
    pub main: u32,
    /// string literal bytes, placed at [`DATA_BASE`]
    pub data: Vec<u8>,
    /// closure function-table: slot -> IR function id
    pub table: Vec<u32>,
    pub uses_print: [bool; 4],
}

/// Addresses below this hold the runtime root slots; address 0 is never an
/// object, so a zero pointer can stand for `None`.
pub const DATA_BASE: u32 = 256;
