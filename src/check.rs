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

use std::collections::{BTreeMap, BTreeSet};

use crate::ast::*;
use crate::diag::{Diagnostic, Pos};

mod generator_lower;
mod scalar_lifetime;
pub mod typed_mir_census;
pub mod lambda_types;
mod direct_lambda;
pub mod callable_ir;

pub const MAX_LOCALS: usize = 1024;
/// Fixed-size i64 continuation slots; all guest generator frames use one size.
pub const GENERATOR_MAX_LIVE_I64_LOCALS: usize = 16;
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
    /// `print_f64(f64)`: the host renders it like Java `Double.toString`
    PrintF64,
    /// `actor_spawn(type: i32) -> i32`: start one actor, return its reference
    ActorSpawn,
    /// `actor_send(ref: i32, value: i64)`: enqueue one message
    ActorSend,
}

impl Import {
    pub const ALL: [Import; 7] = [
        Import::PrintI64,
        Import::PrintBool,
        Import::PrintStr,
        Import::PrintNewline,
        Import::PrintF64,
        Import::ActorSpawn,
        Import::ActorSend,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Import::PrintI64 => "print_i64",
            Import::PrintBool => "print_bool",
            Import::PrintStr => "print_str",
            Import::PrintNewline => "print_newline",
            Import::PrintF64 => "print_f64",
            Import::ActorSpawn => "actor_spawn",
            Import::ActorSend => "actor_send",
        }
    }
}

/// One step of printing an object as `Name{a=1, b=true}` (Java format).
#[derive(Clone, Debug)]
pub enum PrintPiece {
    /// string data at `(offset, len)`
    Str(u32, u32),
    /// field at byte offset, printed as int or bool
    Field(u32, Type),
}

/// String operations lowered to module helpers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StrOp {
    Eq,
    StartsWith,
    EndsWith,
    Contains,
    Trim,
    /// `a + b`: allocates the result on the bump heap
    Concat,
    /// Java `String.compareTo` sign (-1, 0, 1) as an int
    Cmp,
}

#[derive(Clone, Debug)]
pub enum TExpr {
    I64(i64),
    F64(f64),
    Bool(bool),
    /// string value `offset << 32 | len` into the module's string data
    StrConst(u32, u32),
    /// `int` widened to `float` (Java numeric promotion)
    IntToFloat(Box<TE>),
    FNeg(Box<TE>),
    /// string helper call; `Eq` and the predicates take `arg`, `Trim` does not
    Str(StrOp, Box<TE>, Option<Box<TE>>),
    /// print a string value; `tmp` is an i64 scratch local holding it
    PrintStrVal {
        value: Box<TE>,
        tmp: u32,
        newline: bool,
    },
    Local(u32),
    /// Compiler typechecker-attested read through a scalar alias. Wasm
    /// emits the exact same local.get as Local(owner_slot), but the typed
    /// MIR retains this provenance so an audit can reconcile every use.
    LoanRead { loan_id: u32, owner_slot: u32 },
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
    /// enum variant ordinal (i32)
    EnumVal(u32),
    /// Immutable tagged ADT value: 16-byte allocation, i32 tag at 0,
    /// i64 payload at 8; no payload for None. tmp is a fresh i32 local.
    AdtNew { tag: u32, payload: Option<Box<TE>>, tmp: u32 },
    /// Typed prefix trap: the operand is evaluated exactly once before
    /// allocating Some; only the private guest ordinary-throw tag yields None.
    /// All temporaries are function-local to avoid shared error state.
    Trap {
        value: Box<TE>,
        value_tmp: u32,
        some_tmp: u32,
        none_tmp: u32,
    },
    /// Compare the discriminant with a known variant tag.
    AdtIs { obj: Box<TE>, tag: u32 },
    /// Payload read, admissible only inside the tag-matched arm.
    AdtPayload(Box<TE>),
    /// Evaluate a tagged ADT once. On failure return its original handle from
    /// the enclosing function; on success leave the i64 payload on the stack.
    AdtPropagate { source: Box<TE>, tmp: u32, success_tag: u32 },
    /// load a field of a class instance
    FieldGet {
        obj: Box<TE>,
        offset: u32,
    },
    /// read an actor-state global
    GlobalGet(u32),
    /// bump-allocate a zeroed object of `n` bytes; panics when the heap is full
    Alloc(u32),
    /// start an actor of the given type
    Spawn(u32),
    /// enqueue `value` (int, or bool widened to i64) on an actor's mailbox
    Send {
        target: Box<TE>,
        value: Box<TE>,
    },
    /// print an object; `tmp` is an i32 scratch local holding it
    PrintObj {
        obj: Box<TE>,
        tmp: u32,
        pieces: Vec<PrintPiece>,
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
    /// Compiler-checked scalar borrow provenance marker. It emits NO Wasm
    /// instruction, but must survive typed lowering so a lifetime census
    /// cannot silently mistake a borrow for an empty source block.
    /// This does not authorize escaping references or generic loan codegen.
    ScalarBorrowIssue {
        loan_id: u32,
        owner_slot: u32,
        alias: String,
        exclusive: bool,
    },
    /// Compiler-checked alias read/write, ordered before its owning typed statement.
    ScalarLoanUse { loan_id: u32, owner_slot: u32, write: bool },
    /// Non-lexical last-use boundary proven by the existing source checker.
    /// This is a diagnostic fact, not independent permission for codegen.
    ScalarLoanEnd { loan_id: u32, owner_slot: u32 },
    Set(u32, TE),
    /// store `value` into the field at `offset` of `obj`
    FieldSet {
        obj: TE,
        offset: u32,
        ty: Type,
        value: TE,
    },
    /// assign an actor-state global
    GlobalSet(u32, TE),
    Expr(TE),
    /// A guest Error(message) carried by a private Wasm exception tag.
    Throw(TE),
    Return(Option<TE>),
    /// Pull-driven iterator loop; handle is owned by this loop, and closed
    /// at DONE, early break, or before a function-level return.
    ForIn {
        source: TE,
        handle_slot: u32,
        value_slot: u32,
        body: Vec<TStmt>,
    },
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
    /// a path the checker proved impossible (after an exhaustive match)
    Unreachable,
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

/// Typed, bounded suspension instructions for the admitted guest-Wasm profile.
/// Each instruction is addressed by a stable generator resume PC.
#[derive(Clone, Debug)]
pub enum TGenStep {
    Yield(TE),
    Delegate { function: u32, arg: TGenArg },
    CountTo { start: i64 },
    Set { local: u32, value: TE },
    Guard { condition: TE, on_false: u32 },
    Jump { to: u32 },
    Done,
}

#[derive(Clone, Copy, Debug)]
pub enum TGenArg {
    None,
    Literal(i64),
    Parameter,
}

#[derive(Clone, Debug)]
pub struct TGenerator {
    pub function: u32,
    pub steps: Vec<TGenStep>,
    /// Number of arena-resident i64 locals retained across next() calls.
    pub locals: u32,
    /// Bitset of locals live after each resume PC for typed CFG plans.
    /// An empty vector is the legacy CountTo/yield* specialization.
    pub live_after: Vec<u16>,
}

/// One actor type: its state initializers and receive handler.
#[derive(Clone, Debug)]
pub struct TActor {
    pub name: String,
    /// `(state global, initializer)` in declaration order.
    pub inits: Vec<(u32, TE)>,
    /// Function index of `receive(value: i64, sequence: i64)`.
    pub receive: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct TProgram {
    pub funcs: Vec<TFunc>,
    pub generators: Vec<TGenerator>,
    pub main: u32,
    pub imports: Vec<Import>,
    pub data: Vec<u8>,
    /// Classes exist: the module owns a bounded bump heap.
    pub heap: bool,
    /// Types of the actor-state globals, indexed by `TExpr::GlobalGet`.
    pub globals: Vec<Type>,
    pub actors: Vec<TActor>,
    /// String values exist: the module carries memory and the string helpers.
    pub strings: bool,
    /// Emit one module-private Wasm EH tag only when ordinary throw/trap exists.
    pub guest_eh: bool,
}

/// Size in bytes of every field slot in a class instance.
pub const FIELD_BYTES: u32 = 8;

struct FieldInfo {
    name: String,
    ty: Type,
    public: bool,
    mode: FieldMode,
    /// byte offset (classes) or state global (actors)
    slot: u32,
}

#[derive(Clone)]
struct MethodInfo {
    name: String,
    public: bool,
    is_static: bool,
    idx: u32,
    params: Vec<Type>,
    ret: Type,
}

struct ClassInfo {
    name: String,
    fields: Vec<FieldInfo>,
    methods: Vec<MethodInfo>,
    new_idx: u32,
    ctor_idx: Option<u32>,
    ctor_params: Vec<Type>,
}

struct ActorInfo {
    name: String,
    fields: Vec<FieldInfo>,
    methods: Vec<MethodInfo>,
    receive_idx: Option<u32>,
    payload: Type,
}

/// Program-wide declaration tables shared by every body check.
struct Globals<'a> {
    sigs: BTreeMap<(String, usize), (u32, Vec<Type>, Type)>,
    names: BTreeMap<String, Vec<usize>>,
    classes: Vec<ClassInfo>,
    enums: &'a [EnumDecl],
    actors: Vec<ActorInfo>,
    type_names: &'a [(String, Pos)],
}

/// Where the code being checked lives.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Owner {
    TopLevel,
    /// class `c`: `ctor` for the constructor body; static methods have no self
    Class {
        c: u32,
        ctor: bool,
        is_static: bool,
    },
    /// actor `a`; state is reached through `self`
    Actor {
        a: u32,
    },
    /// a field initializer: constant expressions only
    FieldInit {
        actor: bool,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BorrowKind {
    Shared,
    Exclusive,
}

struct Local {
    name: String,
    slot: u32,
    ty: Type,
    mutable: bool,
    /// `Some(payload)` for the `ActorMail<T>` parameter of receive
    mail: Option<Type>,
    /// A compiler-only scalar loan alias; its slot is the source owner's slot.
    borrow: Option<BorrowKind>,
    /// Remains an alias, but its source loan has ended at a compiler-proven
    /// straight-line last-use boundary. A subsequent alias access is rejected.
    borrow_retired: bool,
    /// This conservative NLL slice requires an actual checked alias use.
    /// Unused aliases retain their prior lexical conflict behavior.
    borrow_used: bool,
    /// Distinct compiler-issued identity, never inferred from a mutable slot.
    loan_id: Option<u32>,
}

struct FnCx<'a> {
    g: &'a Globals<'a>,
    owner: Owner,
    ret: Type,
    scopes: Vec<Vec<Local>>,
    locals: Vec<Type>,
    nparams: u32,
    loops: u32,
    /// First slot declared inside each enclosing loop body.
    loop_floors: Vec<u32>,
    /// Class-typed locals whose value has been moved out.
    moved: BTreeSet<u32>,
    /// A single work budget shared by all alias/suffix NLL scans in this function.
    /// Exhaustion simply leaves later loans live until lexical scope exit.
    nll_scan_budget: usize,
    /// Intra-function loan identities, monotonically allocated.
    next_loan_id: u32,
    /// Facts accumulated while typechecking one source statement.
    pending_loan_events: Vec<TStmt>,
    imports: &'a mut Vec<Import>,
    data: &'a mut Vec<u8>,
}

type R<T> = Result<T, Diagnostic>;

fn string_member_err(name: &str, pos: Pos) -> Diagnostic {
    match name {
        "replace_literal" | "split_literal" | "is_identifier" | "is_route_segment" => Diagnostic::new(
            pos,
            "E_UNSUPPORTED",
            format!("string.{name} is not supported by the browser compiler yet"),
        ),
        _ => Diagnostic::new(pos, "E_UNBOUND", format!("unknown string member '{name}'")),
    }
}

fn type_err(pos: Pos, msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new(pos, "E_TYPE", msg)
}

impl Globals<'_> {
    fn tn(&self, t: Type) -> String {
        match t {
            Type::Class(i) => self.classes[i as usize].name.clone(),
            Type::Enum(i) => self.enums[i as usize].name.clone(),
            Type::ActorRef(i) => format!("ActorRef<{}>", self.actors[i as usize].name),
            Type::Named(i) | Type::NamedRef(i) | Type::Callable(i) => self.type_names[i as usize].0.clone(),
            t => t.name().to_string(),
        }
    }

    fn class(&self, name: &str) -> Option<u32> {
        self.classes.iter().position(|c| c.name == name).map(|i| i as u32)
    }
    fn enum_(&self, name: &str) -> Option<u32> {
        self.enums.iter().position(|c| c.name == name).map(|i| i as u32)
    }
    fn actor(&self, name: &str) -> Option<u32> {
        self.actors.iter().position(|c| c.name == name).map(|i| i as u32)
    }
}

/// Resolve parser type names against the declaration tables.
fn resolve(t: Type, type_names: &[(String, Pos)], classes: &[String], enums: &[EnumDecl], actors: &[String]) -> R<Type> {
    Ok(match t {
        Type::Named(i) => {
            let (n, pos) = &type_names[i as usize];
            if let Some(c) = classes.iter().position(|c| c == n) {
                Type::Class(c as u32)
            } else if let Some(e) = enums.iter().position(|e| e.name == *n) {
                Type::Enum(e as u32)
            } else if actors.iter().any(|a| a == n) {
                return Err(type_err(*pos, format!("'{n}' is an actor; its references have type ActorRef<{n}>")));
            } else {
                return Err(Diagnostic::new(*pos, "E_UNKNOWN_TYPE", format!("unknown type '{n}'")));
            }
        }
        Type::Callable(_) => {
            // Type recognition and structural signature validation are
            // available now, but the browser backend has no callable value
            // storage or verified indirect-call ABI. Stop before emitting.
            let signature = callable_ir::CallableSignature::from_source_type(
                t, type_names, Pos { line: 1, col: 1 })?;
            return Err(Diagnostic::new(
                type_names[match t { Type::Callable(i) => i as usize, _ => unreachable!() }].1,
                "E_UNSUPPORTED",
                format!("callable value type {} is parsed and checked but requires validated Wasm indirect dispatch",
                    signature.source_spelling()),
            ));
        }
        Type::NamedRef(i) => {
            let (n, pos) = &type_names[i as usize];
            match actors.iter().position(|a| a == n) {
                Some(a) => Type::ActorRef(a as u32),
                None => return Err(type_err(*pos, format!("ActorRef<{n}> needs an actor type, and '{n}' is not one"))),
            }
        }
        t => t,
    })
}

fn isolation(pos: Pos, actor: &str, what: &str, name: &str) -> Diagnostic {
    Diagnostic::new(
        pos,
        "E_ISOLATION",
        format!(
            "actor '{actor}' cannot implicitly access {what} '{name}'; move the dependency to another source unit and import it explicitly"
        ),
    )
}

/// Field initializers run while the object is under construction: the Java
/// reference forbids calls there, and this compiler admits only constants.
fn const_init(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Int(_) | ExprKind::Bool(_) | ExprKind::Float(_) | ExprKind::Str(_) => true,
        ExprKind::Unary(UnOp::Copy | UnOp::Take | UnOp::Borrow | UnOp::BorrowMut, _) => false,
        ExprKind::Unary(_, a) => const_init(a),
        ExprKind::Binary(_, a, b) => const_init(a) && const_init(b),
        ExprKind::Ternary(c, a, b) => const_init(c) && const_init(a) && const_init(b),
        ExprKind::Member(base, _) => matches!(base.kind, ExprKind::Var(_)),
        _ => false,
    }
}

/// Validate locally decidable generator item/delegation types before the
/// deliberate guest-Wasm continuation gate. This is not full expression
/// inference: unresolved/compound expressions remain fail-closed until the
/// generator IR and ownership-aware checker are implemented.
fn precheck_generator_body(body: &[Stmt], funcs: &[Func]) -> R<()> {
    for stmt in body {
        match stmt {
            Stmt::Yield(expr, _) => {
                if matches!(&expr.kind, ExprKind::Bool(_) | ExprKind::Str(_) | ExprKind::Float(_)) {
                    return Err(Diagnostic::new(expr.pos, "E_TYPE",
                        "Generator<int> can only yield int items"));
                }
            }
            Stmt::YieldFrom(expr, _) => {
                match &expr.kind {
                    ExprKind::Int(_) | ExprKind::Bool(_) | ExprKind::Str(_) | ExprKind::Float(_) => {
                        return Err(Diagnostic::new(expr.pos, "E_TYPE",
                            "yield* requires a generator/iterator, not a scalar"));
                    }
                    ExprKind::Call(name, args) => {
                        if let Some(target) = funcs.iter().find(|f|
                            f.name == *name && f.params.len() == args.len())
                        {
                            if !target.is_generator || target.ret != Type::GeneratorInt {
                                return Err(Diagnostic::new(expr.pos, "E_TYPE",
                                    format!("yield* target '{name}' is not a Generator<int>")));
                            }
                        }
                    }
                    _ => {}
                }
            }
            Stmt::If { arms, otherwise, .. } => {
                for (_, branch) in arms {
                    precheck_generator_body(branch, funcs)?;
                }
                if let Some(branch) = otherwise {
                    precheck_generator_body(branch, funcs)?;
                }
            }
            Stmt::While { body, .. } | Stmt::Loop { body, .. }
            | Stmt::ForIn { body, .. } | Stmt::Block(body) => {
                precheck_generator_body(body, funcs)?;
            }
            Stmt::For { body, .. } => precheck_generator_body(body, funcs)?,
            Stmt::Match { arms, .. } => {
                for arm in arms {
                    precheck_generator_body(&arm.body, funcs)?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// The first executable for-in profile has no exception unwinder for
/// arbitrary user calls inside the loop. Restrict the body to effect-free
/// scalar expressions, local mutations, conditional control-flow, and early
/// return/break/continue. Checked arithmetic traps terminate the Wasm
/// instance, and no guest iterator is allowed to escape that instance.
fn generator_forin_scalar_expr(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Bool(_) |
        ExprKind::Str(_) | ExprKind::Var(_) => true,
        ExprKind::Unary(UnOp::Borrow | UnOp::BorrowMut, _) => false,
        ExprKind::Unary(_, a) => generator_forin_scalar_expr(a),
        ExprKind::Binary(_, a, b) => generator_forin_scalar_expr(a)
            && generator_forin_scalar_expr(b),
        ExprKind::Ternary(a, b, c) => generator_forin_scalar_expr(a)
            && generator_forin_scalar_expr(b) && generator_forin_scalar_expr(c),
        _ => false,
    }
}

fn generator_forin_safe_body(body: &[Stmt]) -> bool {
    body.iter().all(|s| match s {
        Stmt::Bind { init, .. } | Stmt::Assign { value: init, .. } =>
            generator_forin_scalar_expr(init),
        Stmt::If { arms, otherwise, .. } =>
            arms.iter().all(|(cond, branch)|
                generator_forin_scalar_expr(cond) && generator_forin_safe_body(branch))
                && otherwise.as_ref().is_none_or(|b| generator_forin_safe_body(b)),
        Stmt::Block(b) => generator_forin_safe_body(b),
        Stmt::Return(Some(e), _) => generator_forin_scalar_expr(e),
        Stmt::Return(None, _) | Stmt::Break(_) | Stmt::Continue(_) => true,
        _ => false,
    })
}

/// Reject delegation cycles before code generation so an actor cannot
/// recursively allocate child frames without ever yielding an item.
/// The admitted graph is statically resolved and each frame has at most
/// one live delegated child; depth is bounded independently of wasm stacks.
fn validate_generator_graph(gens: &[TGenerator], p: &Program) -> R<()> {
    fn visit(node: usize, depth: u32, colors: &mut [u8],
             gens: &[TGenerator], p: &Program) -> R<()> {
        if colors[node] == 1 {
            return Err(Diagnostic::new(p.funcs[node].pos, "E_GENERATOR_CYCLE",
                "generator yield* delegation must be acyclic"));
        }
        if colors[node] == 2 { return Ok(()); }
        if depth > 64 {
            return Err(Diagnostic::new(p.funcs[node].pos, "E_NESTING",
                "generator delegation chain exceeds 64 frames"));
        }
        colors[node] = 1;
        if let Some(gen) = gens.iter().find(|g| g.function as usize == node) {
            for step in &gen.steps {
                if let TGenStep::Delegate { function, .. } = step {
                    visit(*function as usize, depth + 1, colors, gens, p)?;
                }
            }
        }
        colors[node] = 2;
        Ok(())
    }
    let mut colors = vec![0; p.funcs.len()];
    for g in gens {
        visit(g.function as usize, 1, &mut colors, gens, p)?;
    }
    Ok(())
}

/// Only one immutable int parameter and one mutable counter are admitted.
/// A C-style for must use i <= limit, i++, and yield i without side effects.
fn generator_count_loop(f: &Func, stmt: &Stmt) -> Option<i64> {
    if f.params.len() != 1 || f.params[0].ty != Type::Int { return None; }
    let Stmt::For { init: Some(init), cond: Some(cond),
                    update: Some(update), body, .. } = stmt else { return None; };
    let Stmt::Bind { kind: BindKind::Let, name, ty, init: initial, .. } =
        init.as_ref() else { return None; };
    if ty.is_some_and(|t| t != Type::Int) { return None; }
    let ExprKind::Int(Some(start)) = &initial.kind else { return None; };
    if *start > i64::MAX as u64 { return None; }
    let ExprKind::Binary(BinOp::Le, left, right) = &cond.kind else { return None; };
    if !matches!(&left.kind, ExprKind::Var(s) if s == name)
        || !matches!(&right.kind, ExprKind::Var(s) if s == &f.params[0].name) { return None; }
    let Stmt::Assign { name: update_name, value, .. } = update.as_ref() else { return None; };
    if update_name != name { return None; }
    let ExprKind::Binary(BinOp::Add, left, right) = &value.kind else { return None; };
    if !matches!(&left.kind, ExprKind::Var(s) if s == name)
        || !matches!(&right.kind, ExprKind::Int(Some(1))) { return None; }
    if body.len() != 1 || !matches!(&body[0],
        Stmt::Yield(Expr { kind: ExprKind::Var(s), .. }, _) if s == name) { return None; }
    Some(*start as i64)
}

/// Delegate arguments cannot evaluate arbitrary expressions or alias locals.
fn generator_arg(f: &Func, target: &Func, args: &[Expr]) -> Option<TGenArg> {
    match (target.params.as_slice(), args) {
        ([], []) => Some(TGenArg::None),
        ([p], [arg]) if p.ty == Type::Int => match &arg.kind {
            ExprKind::Int(Some(n)) if *n <= i64::MAX as u64 =>
                Some(TGenArg::Literal(*n as i64)),
            ExprKind::Var(name) if f.params.len() == 1
                && f.params[0].ty == Type::Int && *name == f.params[0].name =>
                Some(TGenArg::Parameter),
            _ => None,
        },
        _ => None,
    }
}

pub fn check(p: &Program) -> R<TProgram> {
    let lowered = direct_lambda::lower(p)?;
    check_lowered(&lowered)
}

fn check_lowered(p: &Program) -> R<TProgram> {
    // Validate the bounded CFG and integer-only suspension/lifetime profile
    // before registering signatures. Reject unsupported generator effects
    // before emission; never substitute eager iterator materialization.
    for f in &p.funcs {
        if f.is_generator {
            precheck_generator_body(&f.body, &p.funcs)?;
            // Build the exact bounded resume graph before the ABI-v2 gate.
            // It is a compiler IR only; no generator guest artifact is emitted.
            let plan = crate::generator_cfg::lower(&f.body)?;
            // Preflight the AST-visible slots. The final codegen must repeat
            // this check using all typed locals and hidden scratch slots.
            let declared_locals = plan.instructions.iter().filter(|op|
                matches!(op, crate::generator_cfg::Instruction::Evaluate(Stmt::Bind { .. }))
            ).count() as u32;
            let _frame = crate::generator_frame::GeneratorFrameLayout::new(
                f.params.len() as u32, declared_locals, plan.delegate_slots, f.pos
            )?;
            // Bounded single-int-parameter and live counter profile.
            let count_loops = f.body.iter().filter(|s| matches!(s, Stmt::For { .. })).count();
            let admitted = f.params.len() <= 1
                && f.params.iter().all(|p| p.ty == Type::Int)
                && count_loops <= 1 && plan.delegate_slots <= 1
                && f.body.iter().enumerate().all(|(i, stmt)| {
                    match stmt {
                        Stmt::Yield(Expr { kind: ExprKind::Int(Some(n)), .. }, _) =>
                            *n <= i64::MAX as u64,
                        Stmt::YieldFrom(Expr { kind: ExprKind::Call(name, args), .. }, _) =>
                            p.funcs.iter().any(|target| target.is_generator
                                && target.name == *name
                                && generator_arg(f, target, args).is_some()),
                        Stmt::For { .. } => generator_count_loop(f, stmt).is_some(),
                        Stmt::Return(None, _) => i + 1 == f.body.len(),
                        _ => false,
                    }
                });
            if !admitted && generator_lower::lower(f)?.is_none() {
                return Err(Diagnostic::new(f.pos, "E_UNSUPPORTED",
                    "generator source requires typed, owned continuation lowering not yet implemented"));
            }
        }
        if (!f.is_generator && f.ret == Type::GeneratorInt)
            || f.params.iter().any(|p| p.ty == Type::GeneratorInt) {
            return Err(Diagnostic::new(f.pos, "E_UNSUPPORTED_TYPE",
                "Generator<int> requires a compiled generator continuation"));
        }
    }
    for c in &p.classes {
        if c.methods.iter().any(|m| m.ret == Type::GeneratorInt
            || m.params.iter().any(|p| p.ty == Type::GeneratorInt))
            || c.ctor.as_ref().is_some_and(|ctor|
                ctor.params.iter().any(|p| p.ty == Type::GeneratorInt)) {
            return Err(Diagnostic::new(c.pos, "E_UNSUPPORTED_TYPE",
                "generator methods and parameters require a compiled iterator continuation"));
        }
    }
    for a in &p.actors {
        if a.methods.iter().any(|m| m.ret == Type::GeneratorInt
            || m.params.iter().any(|p| p.ty == Type::GeneratorInt)) {
            return Err(Diagnostic::new(a.pos, "E_UNSUPPORTED_TYPE",
                "actor generator methods require a compiled iterator continuation"));
        }
    }
    let tn = &p.type_names;
    // ---- one namespace for functions, classes, enums and actors
    let mut seen: BTreeMap<&str, Pos> = BTreeMap::new();
    let decl_names = p
        .classes
        .iter()
        .map(|c| (c.name.as_str(), c.pos, "class"))
        .chain(p.enums.iter().map(|e| (e.name.as_str(), e.pos, "enum")))
        .chain(p.actors.iter().map(|a| (a.name.as_str(), a.pos, "actor")));
    for (name, pos, kind) in decl_names {
        if matches!(
            name,
            "And" | "Or" | "Xor" | "stdio" | "ActorRef" | "ActorMail" | "int" | "bool" | "boolean"
        ) {
            return Err(Diagnostic::new(
                pos,
                "E_DUPLICATE",
                format!("'{name}' is a builtin and cannot be redefined"),
            ));
        }
        if seen.insert(name, pos).is_some() {
            return Err(Diagnostic::new(
                pos,
                "E_DUPLICATE",
                format!("duplicate type name '{name}' ({kind})"),
            ));
        }
    }
    for f in &p.funcs {
        if seen.contains_key(f.name.as_str()) {
            return Err(Diagnostic::new(
                f.pos,
                "E_DUPLICATE",
                format!("'{}' is already declared as a type", f.name),
            ));
        }
    }
    let class_names: Vec<String> = p.classes.iter().map(|c| c.name.clone()).collect();
    let actor_names: Vec<String> = p.actors.iter().map(|a| a.name.clone()).collect();
    let res = |t: Type| resolve(t, tn, &class_names, &p.enums, &actor_names);

    // ---- top-level signatures keep indices 0..funcs.len()
    let mut sigs = BTreeMap::new();
    let mut names: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, f) in p.funcs.iter().enumerate() {
        if matches!(f.name.as_str(), "And" | "Or" | "Xor" | "stdio" | "Ok" | "Err" | "Some" | "None") {
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
        let params = f.params.iter().map(|q| res(q.ty)).collect::<R<Vec<_>>>()?;
        sigs.insert(key, (i as u32, params, res(f.ret)?));
        names.entry(f.name.clone()).or_default().push(f.params.len());
    }

    // ---- member function indices follow the top-level functions
    let mut next = p.funcs.len() as u32;
    let mut take = || {
        next += 1;
        next - 1
    };
    let methods_of = |owner: &str, ms: &[Method], fields: &[Field]| -> R<Vec<MethodInfo>> {
        let mut out: Vec<MethodInfo> = Vec::new();
        for m in ms {
            if out.iter().any(|o| o.name == m.name) || fields.iter().any(|f| f.name == m.name) {
                return Err(Diagnostic::new(
                    m.pos,
                    "E_DUPLICATE",
                    format!("'{owner}' already declares a member named '{}'", m.name),
                ));
            }
            out.push(MethodInfo {
                name: m.name.clone(),
                public: m.public,
                is_static: m.is_static,
                idx: 0,
                params: m.params.iter().map(|q| res(q.ty)).collect::<R<Vec<_>>>()?,
                ret: res(m.ret)?,
            });
        }
        Ok(out)
    };
    let fields_of = |owner: &str, fs: &[Field], actor: bool, global_base: u32| -> R<Vec<FieldInfo>> {
        let mut out: Vec<FieldInfo> = Vec::new();
        for (i, f) in fs.iter().enumerate() {
            if out.iter().any(|o| o.name == f.name) {
                return Err(Diagnostic::new(
                    f.pos,
                    "E_DUPLICATE",
                    format!("'{owner}' already declares a field named '{}'", f.name),
                ));
            }
            let ty = res(f.ty)?;
            let ok = match ty {
                Type::Int | Type::Bool => true,
                Type::Float | Type::Str | Type::Enum(_) => !actor,
                _ => false,
            };
            if !ok {
                return Err(Diagnostic::new(
                    f.pos,
                    "E_UNSUPPORTED_TYPE",
                    format!(
                        "field '{owner}.{}' has type {}; {} fields may be int or bool{} in the browser compiler",
                        f.name,
                        tn_plain(ty, tn, &class_names, &p.enums, &actor_names),
                        if actor { "actor" } else { "class" },
                        if actor { "" } else { ", float, string or an enum" }
                    ),
                ));
            }
            out.push(FieldInfo {
                name: f.name.clone(),
                ty,
                public: f.public,
                mode: f.mode,
                slot: if actor { global_base + i as u32 } else { i as u32 * FIELD_BYTES },
            });
        }
        Ok(out)
    };

    let mut classes = Vec::new();
    for c in &p.classes {
        let fields = fields_of(&c.name, &c.fields, false, 0)?;
        let mut methods = methods_of(&c.name, &c.methods, &c.fields)?;
        let new_idx = take();
        let ctor_idx = c.ctor.as_ref().map(|_| take());
        for m in &mut methods {
            m.idx = take();
        }
        let ctor_params = match &c.ctor {
            Some(k) => k.params.iter().map(|q| res(q.ty)).collect::<R<Vec<_>>>()?,
            None => Vec::new(),
        };
        classes.push(ClassInfo {
            name: c.name.clone(),
            fields,
            methods,
            new_idx,
            ctor_idx,
            ctor_params,
        });
    }
    let mut actors = Vec::new();
    let mut globals = Vec::new();
    for a in &p.actors {
        let fields = fields_of(&a.name, &a.fields, true, globals.len() as u32)?;
        globals.extend(fields.iter().map(|f| f.ty));
        let mut methods = methods_of(&a.name, &a.methods, &a.fields)?;
        if let Some(m) = a.methods.iter().find(|m| m.name == "receive") {
            return Err(Diagnostic::new(
                m.pos,
                "E_TYPE",
                format!("actor mailbox handler '{}.receive' parameter must be ActorMail<T>", a.name),
            ));
        }
        let receive_idx = a.receive.as_ref().map(|_| take());
        for m in &mut methods {
            m.idx = take();
        }
        let payload = match &a.receive {
            Some(r) => {
                let t = res(r.payload)?;
                if !matches!(t, Type::Int | Type::Bool) {
                    return Err(Diagnostic::new(
                        r.pos,
                        "E_UNSUPPORTED_TYPE",
                        format!(
                            "ActorMail<{}> is not supported by the browser compiler yet; mail payloads may be int or bool",
                            tn_plain(t, tn, &class_names, &p.enums, &actor_names)
                        ),
                    ));
                }
                t
            }
            None => Type::Void,
        };
        actors.push(ActorInfo {
            name: a.name.clone(),
            fields,
            methods,
            receive_idx,
            payload,
        });
    }
    let total = next as usize;

    let g = Globals {
        sigs,
        names,
        classes,
        enums: &p.enums,
        actors,
        type_names: tn,
    };

    let main = match g.sigs.get(&("main".to_string(), 0)) {
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
    let mut funcs: Vec<Option<TFunc>> = (0..total).map(|_| None).collect();

    // ---- top-level functions
    let mut generators = Vec::new();
    for (i, f) in p.funcs.iter().enumerate() {
        let (_, params, ret) = g.sigs[&(f.name.clone(), f.params.len())].clone();
        if f.is_generator {
            if let Some(mut plan) = generator_lower::lower(f)? {
                plan.function = i as u32;
                generators.push(plan);
                funcs[i] = Some(TFunc {
                    name: f.name.clone(), params, ret,
                    locals: Vec::new(), body: Vec::new(),
                });
                continue;
            }
            // Original bounded CountTo/direct yield* fallback.
            let steps = f.body.iter().filter_map(|stmt| match stmt {
                Stmt::Yield(Expr { kind: ExprKind::Int(Some(n)), .. }, _) =>
                    Some(TGenStep::Yield(TE {
                        e: TExpr::I64(*n as i64), ty: Type::Int
                    })),
                Stmt::YieldFrom(Expr { kind: ExprKind::Call(name, args), .. }, _) => {
                    let target = p.funcs.iter().find(|target| target.is_generator
                        && target.name == *name && target.params.len() == args.len())
                        .expect("source validated before declaration lowering");
                    let arg = generator_arg(f, target, args)
                        .expect("argument validated before declaration lowering");
                    let idx = g.sigs.get(&(name.clone(), args.len()))
                        .expect("declared generator target").0;
                    Some(TGenStep::Delegate { function: idx, arg })
                }
                Stmt::For { .. } => Some(TGenStep::CountTo {
                    start: generator_count_loop(f, stmt).expect("verified counter loop")
                }),
                _ => None,
            }).collect();
            generators.push(TGenerator { function: i as u32, steps, locals: 0, live_after: Vec::new() });
            funcs[i] = Some(TFunc {
                name: f.name.clone(), params, ret,
                locals: Vec::new(), body: Vec::new(),
            });
            continue;
        }
        let named: Vec<(&str, Type, bool)> = f
            .params
            .iter()
            .zip(&params)
            .map(|(q, t)| (q.name.as_str(), *t, q.mutable))
            .collect();
        let body = body_check(
            &g,
            Owner::TopLevel,
            &named,
            None,
            ret,
            &f.body,
            &f.name,
            f.pos,
            &mut imports,
            &mut data,
        )?;
        funcs[i] = Some(TFunc {
            name: f.name.clone(),
            params,
            ret,
            locals: body.1,
            body: body.0,
        });
    }

    // ---- classes: P.new, P.constructor, methods
    for (ci, c) in p.classes.iter().enumerate() {
        let info = &g.classes[ci];
        let me = Type::Class(ci as u32);
        let mut field_inits = Vec::new();
        for (f, fi) in c.fields.iter().zip(&info.fields) {
            let v = field_init(&g, f, fi.ty, &c.name, false, &mut imports, &mut data)?;
            field_inits.push((fi.slot, fi.ty, v));
        }
        if let (Some(k), Some(kidx)) = (&c.ctor, info.ctor_idx) {
            let mut named = vec![("self", me, false)];
            named.extend(
                k.params
                    .iter()
                    .zip(&info.ctor_params)
                    .map(|(q, t)| (q.name.as_str(), *t, q.mutable)),
            );
            let owner = Owner::Class {
                c: ci as u32,
                ctor: true,
                is_static: false,
            };
            let body = body_check(
                &g,
                owner,
                &named,
                None,
                Type::Void,
                &k.body,
                "constructor",
                k.pos,
                &mut imports,
                &mut data,
            )?;
            let mut params = vec![me];
            params.extend(info.ctor_params.iter().copied());
            funcs[kidx as usize] = Some(TFunc {
                name: format!("{}.constructor", c.name),
                params,
                ret: Type::Void,
                locals: body.1,
                body: body.0,
            });
        }
        // P.new(args): allocate, run field initializers, run the constructor
        let n = info.ctor_params.len() as u32;
        let obj = TE {
            e: TExpr::Local(n),
            ty: me,
        };
        let mut body = vec![TStmt::Set(
            n,
            TE {
                e: TExpr::Alloc((info.fields.len() as u32 * FIELD_BYTES).max(FIELD_BYTES)),
                ty: me,
            },
        )];
        for (offset, ty, v) in field_inits {
            body.push(TStmt::FieldSet {
                obj: obj.clone(),
                offset,
                ty,
                value: v,
            });
        }
        if let Some(kidx) = info.ctor_idx {
            let mut args = vec![obj.clone()];
            args.extend(info.ctor_params.iter().enumerate().map(|(i, t)| TE {
                e: TExpr::Local(i as u32),
                ty: *t,
            }));
            body.push(TStmt::Expr(TE {
                e: TExpr::Call(kidx, args),
                ty: Type::Void,
            }));
        }
        body.push(TStmt::Return(Some(obj)));
        funcs[info.new_idx as usize] = Some(TFunc {
            name: format!("{}.new", c.name),
            params: info.ctor_params.clone(),
            ret: me,
            locals: vec![me],
            body,
        });
        for (m, mi) in c.methods.iter().zip(&info.methods) {
            let owner = Owner::Class {
                c: ci as u32,
                ctor: false,
                is_static: m.is_static,
            };
            let mut named = Vec::new();
            if !m.is_static {
                named.push(("self", me, false));
            }
            named.extend(m.params.iter().zip(&mi.params).map(|(q, t)| (q.name.as_str(), *t, q.mutable)));
            let label = format!("{}.{}", c.name, m.name);
            let body = body_check(&g, owner, &named, None, mi.ret, &m.body, &label, m.pos, &mut imports, &mut data)?;
            funcs[mi.idx as usize] = Some(TFunc {
                name: label,
                params: named.iter().map(|x| x.1).collect(),
                ret: mi.ret,
                locals: body.1,
                body: body.0,
            });
        }
    }

    // ---- actors: state initializers, receive, methods
    let mut tactors = Vec::new();
    for (ai, a) in p.actors.iter().enumerate() {
        let info = &g.actors[ai];
        let owner = Owner::Actor { a: ai as u32 };
        let mut inits = Vec::new();
        for (f, fi) in a.fields.iter().zip(&info.fields) {
            inits.push((fi.slot, field_init(&g, f, fi.ty, &a.name, true, &mut imports, &mut data)?));
        }
        if let (Some(r), Some(ridx)) = (&a.receive, info.receive_idx) {
            let label = format!("{}.receive", a.name);
            let named = [("$value", Type::Int, false), ("$sequence", Type::Int, false)];
            let body = body_check(
                &g,
                owner,
                &named,
                Some((r.mail.as_str(), info.payload)),
                Type::Void,
                &r.body,
                &label,
                r.pos,
                &mut imports,
                &mut data,
            )?;
            funcs[ridx as usize] = Some(TFunc {
                name: label,
                params: vec![Type::Int, Type::Int],
                ret: Type::Void,
                locals: body.1,
                body: body.0,
            });
        }
        for (m, mi) in a.methods.iter().zip(&info.methods) {
            for (q, t) in m.params.iter().zip(&mi.params).map(|(q, t)| (q, *t)).chain(std::iter::once((
                &Param {
                    name: String::new(),
                    ty: mi.ret,
                    mutable: false,
                    pos: m.pos,
                },
                mi.ret,
            ))) {
                actor_type_ok(&g, &a.name, t, q.pos)?;
            }
            let named: Vec<(&str, Type, bool)> = m
                .params
                .iter()
                .zip(&mi.params)
                .map(|(q, t)| (q.name.as_str(), *t, q.mutable))
                .collect();
            let label = format!("{}.{}", a.name, m.name);
            let body = body_check(&g, owner, &named, None, mi.ret, &m.body, &label, m.pos, &mut imports, &mut data)?;
            funcs[mi.idx as usize] = Some(TFunc {
                name: label,
                params: mi.params.clone(),
                ret: mi.ret,
                locals: body.1,
                body: body.0,
            });
        }
        tactors.push(TActor {
            name: a.name.clone(),
            inits,
            receive: info.receive_idx,
        });
    }

    validate_generator_graph(&generators, p)?;
    imports.sort();
    imports.dedup();
    let funcs: Vec<TFunc> = funcs.into_iter().map(|f| f.expect("every function index is defined")).collect();
    let strings = funcs
        .iter()
        .any(|f| f.params.contains(&Type::Str) || f.ret == Type::Str || f.locals.contains(&Type::Str) || any_in_stmts(&f.body, &uses_str))
        || tactors.iter().any(|a| a.inits.iter().any(|(_, e)| any_in(e, &uses_str)));
    let concat = funcs.iter().any(|f| any_in_stmts(&f.body, &uses_concat));
    let adt_alloc = funcs.iter().any(|f| any_in_stmts(&f.body,
        &|e| matches!(e.e, TExpr::AdtNew { .. } | TExpr::Trap { .. })));
    let guest_eh = funcs.iter().any(|f| guest_eh_in_stmts(&f.body)
        || any_in_stmts(&f.body, &|e| matches!(e.e, TExpr::Trap { .. })));
    Ok(TProgram {
        funcs,
        main,
        imports,
        data,
        heap: !p.classes.is_empty() || concat || adt_alloc || !generators.is_empty(),
        generators,
        globals,
        actors: tactors,
        strings,
        guest_eh,
    })
}

fn tn_plain(t: Type, tn: &[(String, Pos)], classes: &[String], enums: &[EnumDecl], actors: &[String]) -> String {
    match t {
        Type::Class(i) => classes[i as usize].clone(),
        Type::Enum(i) => enums[i as usize].name.clone(),
        Type::ActorRef(i) => format!("ActorRef<{}>", actors[i as usize]),
        Type::Named(i) | Type::NamedRef(i) | Type::Callable(i) => tn[i as usize].0.clone(),
        t => t.name().to_string(),
    }
}

/// Actors are isolated from sibling classes, enums and other actors.
fn actor_type_ok(g: &Globals, actor: &str, t: Type, pos: Pos) -> R<()> {
    match t {
        Type::Class(i) => Err(isolation(pos, actor, "sibling class", &g.classes[i as usize].name)),
        Type::Enum(i) => Err(isolation(pos, actor, "sibling enum", &g.enums[i as usize].name)),
        Type::ActorRef(i) => Err(isolation(pos, actor, "actor", &g.actors[i as usize].name)),
        _ => Ok(()),
    }
}

fn field_init(g: &Globals, f: &Field, ty: Type, owner: &str, actor: bool, imports: &mut Vec<Import>, data: &mut Vec<u8>) -> R<TE> {
    if !const_init(&f.init) {
        return Err(Diagnostic::new(
            f.init.pos,
            "E_UNSUPPORTED",
            format!(
                "field initializer '{owner}.{}' must be a constant expression; initialize computed values in the constructor",
                f.name
            ),
        ));
    }
    let mut cx = FnCx::new(g, Owner::FieldInit { actor }, Type::Void, imports, data);
    let v = cx.value(&f.init)?;
    if v.ty != ty {
        return Err(type_err(
            f.init.pos,
            format!(
                "field '{owner}.{}' has type {} but its initializer has type {}",
                f.name,
                g.tn(ty),
                g.tn(v.ty)
            ),
        ));
    }
    Ok(v)
}

/// Check one callable body. `named` are the wasm parameters in order.
#[allow(clippy::too_many_arguments)]
fn body_check(
    g: &Globals,
    owner: Owner,
    named: &[(&str, Type, bool)],
    mail: Option<(&str, Type)>,
    ret: Type,
    body: &[Stmt],
    label: &str,
    pos: Pos,
    imports: &mut Vec<Import>,
    data: &mut Vec<u8>,
) -> R<(Vec<TStmt>, Vec<Type>)> {
    let mut cx = FnCx::new(g, owner, ret, imports, data);
    cx.nparams = named.len() as u32;
    if let Some((mail_name, payload)) = mail {
        cx.scopes[0].push(Local {
            name: mail_name.to_string(),
            slot: 0,
            ty: Type::Int,
            mutable: false,
            mail: Some(payload),
            borrow: None,
            borrow_retired: false,
            borrow_used: false,
            loan_id: None,
        });
    } else {
        for (i, (n, t, m)) in named.iter().enumerate() {
            cx.scopes[0].push(Local {
                name: n.to_string(),
                slot: i as u32,
                ty: *t,
                mutable: *m,
                mail: None,
                borrow: None,
                borrow_retired: false,
            borrow_used: false,
            loan_id: None,
            });
        }
    }
    cx.scopes.push(Vec::new());
    let tbody = cx.stmts(body)?;
    if ret != Type::Void && !returns(&tbody) {
        let what = if matches!(owner, Owner::TopLevel) { "fnc" } else { "method" };
        return Err(Diagnostic::new(
            pos,
            "E_RETURN",
            format!("non-void {what} '{label}' must explicitly return on every path"),
        ));
    }
    Ok((tbody, cx.locals))
}

/// Definite-return analysis matching the Java reference: an `if` returns
/// only with an `else` whose every arm returns; an unconditional `loop`
/// without a `break` never falls through; `while true` is not special.
fn returns(body: &[TStmt]) -> bool {
    body.iter().any(|s| match s {
        TStmt::Return(_) | TStmt::Throw(_) | TStmt::Unreachable => true,
        TStmt::If(arms, Some(other)) => arms.iter().all(|(_, b)| returns(b)) && returns(other),
        TStmt::Loop { cond: None, body, .. } => !breaks(body),
        TStmt::Block(b) => returns(b),
        _ => false,
    })
}

/// Whether any expression node in `body` satisfies `f`.
fn any_in_stmts(body: &[TStmt], f: &dyn Fn(&TE) -> bool) -> bool {
    body.iter().any(|s| match s {
        TStmt::Set(_, e) | TStmt::GlobalSet(_, e) | TStmt::Expr(e) | TStmt::Throw(e) | TStmt::Return(Some(e)) => any_in(e, f),
        TStmt::FieldSet { obj, value, .. } => any_in(obj, f) || any_in(value, f),
        TStmt::If(arms, other) => {
            arms.iter().any(|(c, b)| any_in(c, f) || any_in_stmts(b, f)) || other.as_ref().is_some_and(|b| any_in_stmts(b, f))
        }
        TStmt::Loop { cond, body, update } => {
            cond.as_ref().is_some_and(|c| any_in(c, f)) || any_in_stmts(body, f) || any_in_stmts(update, f)
        }
        TStmt::ForIn { source, body, .. } => any_in(source, f) || any_in_stmts(body, f),
        TStmt::Block(b) => any_in_stmts(b, f),
        TStmt::Return(None) | TStmt::Break | TStmt::Continue | TStmt::Unreachable
        | TStmt::ScalarBorrowIssue { .. }
        | TStmt::ScalarLoanUse { .. } | TStmt::ScalarLoanEnd { .. } => false,
    })
}

fn any_in(e: &TE, f: &dyn Fn(&TE) -> bool) -> bool {
    if f(e) {
        return true;
    }
    match &e.e {
        TExpr::Trap { value, .. } => any_in(value, f),
        TExpr::Neg(a) | TExpr::Not(a) | TExpr::FNeg(a) | TExpr::IntToFloat(a) => any_in(a, f),
        TExpr::Bin(_, a, b) | TExpr::Builtin(_, a, b) => any_in(a, f) || any_in(b, f),
        TExpr::Ternary(c, a, b) => any_in(c, f) || any_in(a, f) || any_in(b, f),
        TExpr::Str(_, a, b) => any_in(a, f) || b.as_ref().is_some_and(|b| any_in(b, f)),
        TExpr::Call(_, args) => args.iter().any(|a| any_in(a, f)),
        TExpr::AdtNew { payload, .. } => payload.as_ref().is_some_and(|p| any_in(p, f)),
        TExpr::AdtIs { obj, .. } | TExpr::AdtPayload(obj) => any_in(obj, f),
        TExpr::AdtPropagate { source, .. } => any_in(source, f),
        TExpr::Print { value, .. } | TExpr::PrintStrVal { value, .. } => any_in(value, f),
        TExpr::FieldGet { obj, .. } | TExpr::PrintObj { obj, .. } => any_in(obj, f),
        TExpr::Send { target, value } => any_in(target, f) || any_in(value, f),
        _ => false,
    }
}

/// An actual ordinary throw, including one nested in a conditional or loop.
fn guest_eh_in_stmts(body: &[TStmt]) -> bool {
    body.iter().any(|s| match s {
        TStmt::Throw(_) => true,
        TStmt::If(arms, other) => arms.iter().any(|(_, b)| guest_eh_in_stmts(b))
            || other.as_ref().is_some_and(|b| guest_eh_in_stmts(b)),
        TStmt::Loop { body, update, .. } =>
            guest_eh_in_stmts(body) || guest_eh_in_stmts(update),
        TStmt::ForIn { body, .. } => guest_eh_in_stmts(body),
        TStmt::Block(b) => guest_eh_in_stmts(b),
        _ => false,
    })
}

fn uses_str(e: &TE) -> bool {
    e.ty == Type::Str
        || matches!(e.e, TExpr::Str(..) | TExpr::StrConst(..) | TExpr::PrintStrVal { .. })
        || matches!(&e.e, TExpr::PrintObj { pieces, .. } if pieces.iter().any(|p| matches!(p, PrintPiece::Field(_, Type::Str))))
}

fn uses_concat(e: &TE) -> bool {
    matches!(e.e, TExpr::Str(StrOp::Concat, ..))
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

impl<'a> FnCx<'a> {
    fn new(g: &'a Globals<'a>, owner: Owner, ret: Type, imports: &'a mut Vec<Import>, data: &'a mut Vec<u8>) -> Self {
        FnCx {
            g,
            owner,
            ret,
            scopes: vec![Vec::new()],
            locals: Vec::new(),
            nparams: 0,
            loops: 0,
            loop_floors: Vec::new(),
            moved: BTreeSet::new(),
            nll_scan_budget: scalar_lifetime::MAX_NLL_WORK_PER_FUNCTION,
            next_loan_id: 1,
            pending_loan_events: Vec::new(),
            imports,
            data,
        }
    }

    fn tn(&self, t: Type) -> String {
        self.g.tn(t)
    }

    fn actor_name(&self) -> Option<&str> {
        match self.owner {
            Owner::Actor { a } => Some(&self.g.actors[a as usize].name),
            _ => None,
        }
    }

    fn lookup(&self, name: &str) -> Option<&Local> {
        self.scopes.iter().rev().flat_map(|s| s.iter().rev()).find(|l| l.name == name)
    }

    fn resolve(&self, t: Type, pos: Pos) -> R<Type> {
        let classes: Vec<String> = self.g.classes.iter().map(|c| c.name.clone()).collect();
        let actors: Vec<String> = self.g.actors.iter().map(|a| a.name.clone()).collect();
        let t = resolve(t, self.g.type_names, &classes, self.g.enums, &actors)?;
        if let Some(a) = self.actor_name() {
            actor_type_ok(self.g, a, t, pos)?;
        }
        Ok(t)
    }

    fn declare(&mut self, name: &str, ty: Type, mutable: bool, pos: Pos) -> R<u32> {
        if self.scopes.last().unwrap().iter().any(|l| l.name == name) {
            return Err(Diagnostic::new(pos, "E_DUPLICATE", format!("duplicate binding '{name}'")));
        }
        if name == "self" {
            return Err(Diagnostic::new(pos, "E_RESERVED", "'self' cannot be rebound"));
        }
        let slot = self.temp(ty, pos)?;
        self.scopes.last_mut().unwrap().push(Local {
            name: name.to_string(),
            slot,
            ty,
            mutable,
            mail: None,
            borrow: None,
            borrow_retired: false,
            borrow_used: false,
            loan_id: None,
        });
        Ok(slot)
    }

    /// A fresh unnamed local.
    fn temp(&mut self, ty: Type, pos: Pos) -> R<u32> {
        if self.locals.len() >= MAX_LOCALS {
            return Err(Diagnostic::new(
                pos,
                "E_SOURCE_LIMIT",
                format!("more than {MAX_LOCALS} locals in one function"),
            ));
        }
        let slot = self.nparams + self.locals.len() as u32;
        self.locals.push(ty);
        Ok(slot)
    }

    /// Loan records live in lexical scopes and disappear on scope exit,
    /// including a branch-specific scope. No guest reference is manufactured.
    fn loan_state(&self, owner_slot: u32) -> (usize, bool) {
        let mut count = 0;
        let mut exclusive = false;
        for local in self.scopes.iter().flatten() {
            if local.slot == owner_slot {
                if let Some(kind) = local.borrow {
                    if !local.borrow_retired {
                        count += 1;
                        exclusive |= kind == BorrowKind::Exclusive;
                    }
                }
            }
        }
        (count, exclusive)
    }

    fn ensure_unloaned(&self, slot: u32, pos: Pos) -> R<()> {
        if self.loan_state(slot).0 != 0 {
            return Err(Diagnostic::new(pos, "E_BORROW",
                "cannot mutate or move the owner while a borrow is live"));
        }
        Ok(())
    }

    #[inline(never)]
    fn bind_scalar_borrow(&mut self, kind: BindKind, name: &str, annotation: Option<Type>,
        inner: &Expr, exclusive: bool, pos: Pos) -> R<TStmt> {
        if kind != BindKind::Val || annotation.is_some() {
            return Err(Diagnostic::new(pos, "E_UNSUPPORTED",
                "scalar borrow aliases currently require an inferred 'val name = borrow! ...' binding"));
        }
        let ExprKind::Var(owner_name) = &inner.kind else {
            return Err(Diagnostic::new(inner.pos, "E_BORROW",
                "borrow! requires a named, live scalar local"));
        };
        let owner = match self.lookup(owner_name) {
            Some(local) => local,
            None => return Err(Diagnostic::new(inner.pos, "E_UNBOUND",
                format!("unknown borrow owner '{owner_name}'"))),
        };
        let (slot, ty, mutable, existing_loan, mail) =
            (owner.slot, owner.ty, owner.mutable, owner.borrow, owner.mail);
        if mail.is_some() {
            return Err(Diagnostic::new(inner.pos, "E_UNSUPPORTED",
                "ActorMail envelopes cannot be borrowed as scalar locals; read the message payload explicitly"));
        }
        if existing_loan.is_some() {
            return Err(Diagnostic::new(inner.pos, "E_BORROW",
                "reborrowing an alias requires explicit lifetime/provenance lowering"));
        }
        if self.moved.contains(&slot) {
            return Err(Diagnostic::new(inner.pos, "E_MOVED",
                format!("cannot borrow moved owner '{owner_name}'")));
        }
        if !matches!(ty, Type::Int | Type::Bool | Type::Float | Type::Str) {
            return Err(Diagnostic::new(inner.pos, "E_UNSUPPORTED",
                "only nonescaping scalar local loans are supported; heap and actor loans require ownership lowering"));
        }
        if exclusive && !mutable {
            return Err(Diagnostic::new(inner.pos, "E_MUTABILITY",
                "borrow! mut requires a mutable owner binding"));
        }
        let (count, has_exclusive) = self.loan_state(slot);
        if has_exclusive || (exclusive && count > 0) {
            return Err(Diagnostic::new(inner.pos, "E_BORROW",
                "borrow conflicts with an existing shared or exclusive loan"));
        }
        if self.scopes.last().unwrap().iter().any(|l| l.name == name) {
            return Err(Diagnostic::new(pos, "E_DUPLICATE", format!("duplicate binding '{name}'")));
        }
        if name == "self" {
            return Err(Diagnostic::new(pos, "E_RESERVED", "'self' cannot be rebound"));
        }
        // A borrow uses no guest-local slot, so enforce the same bounded
        // live-name budget explicitly (MAX_LOCALS) to avoid quadratic scans.
        if self.scopes.iter().map(Vec::len).sum::<usize>() >= MAX_LOCALS {
            return Err(Diagnostic::new(pos, "E_SOURCE_LIMIT",
                "too many simultaneously live local names and borrow aliases"));
        }
        let loan_id = self.next_loan_id;
        self.next_loan_id = self.next_loan_id.checked_add(1).ok_or_else(|| {
            Diagnostic::new(pos, "E_SOURCE_LIMIT", "too many checked scalar loan identities")
        })?;
        self.scopes.last_mut().unwrap().push(Local {
            name: name.to_string(), slot, ty, mutable: false, mail: None,
            borrow: Some(if exclusive { BorrowKind::Exclusive } else { BorrowKind::Shared }),
            borrow_retired: false,
            borrow_used: false,
            loan_id: Some(loan_id),
        });
        // The loan is a source-level reference, not a copy. The read and
        // write-through operations subsequently lower directly to the owner's
        // checked scalar local, with no guest pointer or heap alias.
        Ok(TStmt::ScalarBorrowIssue {
            loan_id,
            owner_slot: slot,
            alias: name.to_string(),
            exclusive,
        })
    }

    fn scoped(&mut self, body: &[Stmt]) -> R<Vec<TStmt>> {
        self.scopes.push(Vec::new());
        let r = self.stmts(body);
        self.scopes.pop();
        r
    }

    fn looped(&mut self, body: &[Stmt]) -> R<Vec<TStmt>> {
        self.loops += 1;
        self.loop_floors.push(self.nparams + self.locals.len() as u32);
        let r = self.scoped(body);
        self.loop_floors.pop();
        self.loops -= 1;
        r
    }

    /// End a scalar loan only after independently scanning *every* remaining
    /// straight-line AST statement for future uses. Restricted to aliases
    /// declared in THIS scope; outer aliases may be used after this block.
    /// No loop backedge, exception, async, or unknown control flow is inferred.
    fn mark_scalar_alias_used(&mut self, name: &str, write: bool) {
        if let Some(local) = self.scopes.iter_mut().rev()
            .flat_map(|scope| scope.iter_mut().rev())
            .find(|local| local.name == name && local.borrow.is_some())
        {
            local.borrow_used = true;
            if let Some(loan_id) = local.loan_id {
                self.pending_loan_events.push(TStmt::ScalarLoanUse {
                    loan_id,
                    owner_slot: local.slot,
                    write,
                });
            }
        }
    }

    fn retire_dead_scalar_loans(&mut self, tail: &[Stmt]) {
        if self.loops != 0 || self.scopes.last().is_none_or(|s| s.len() > 32) {
            return;
        }
        let current = self.scopes.last_mut().expect("active scope");
        for local in current {
            if local.borrow.is_some()
                && local.borrow_used
                && !local.borrow_retired
                && scalar_lifetime::proven_dead_after_boundary(
                    tail, &local.name, &mut self.nll_scan_budget
                )
            {
                local.borrow_retired = true;
                if let Some(loan_id) = local.loan_id {
                    self.pending_loan_events.push(TStmt::ScalarLoanEnd {
                        loan_id,
                        owner_slot: local.slot,
                    });
                }
            }
        }
    }

    fn stmts(&mut self, body: &[Stmt]) -> R<Vec<TStmt>> {
        let mut out = Vec::with_capacity(body.len());
        for (index, statement) in body.iter().enumerate() {
            self.retire_dead_scalar_loans(&body[index..]);
            out.append(&mut self.pending_loan_events);
            let typed = self.stmt(statement)?;
            out.append(&mut self.pending_loan_events);
            out.push(typed);
        }
        Ok(out)
    }

    fn cond(&mut self, e: &Expr, what: &str) -> R<TE> {
        let c = self.expr(e)?;
        if c.ty != Type::Bool {
            return Err(type_err(
                e.pos,
                format!("{what} condition must have type bool, found {}", self.tn(c.ty)),
            ));
        }
        Ok(c)
    }

    fn stmt(&mut self, s: &Stmt) -> R<TStmt> {
        Ok(match s {
            Stmt::Bind { kind, name, ty, init, pos } => {
                if let ExprKind::Unary(op @ (UnOp::Borrow | UnOp::BorrowMut), inner) = &init.kind {
                    return self.bind_scalar_borrow(*kind, name, *ty, inner,
                        *op == UnOp::BorrowMut, *pos);
                }
                let want = match ty {
                    Some(t) => Some(self.resolve(*t, *pos)?),
                    None => None,
                };
                let v = match want {
                    Some(t) => self.consume_expected(init, t)?,
                    None => self.consume(init)?,
                };
                if let Some(t) = want {
                    if t != v.ty {
                        return Err(type_err(
                            init.pos,
                            format!(
                                "binding '{name}' has type {} but its initializer has type {}",
                                self.tn(t),
                                self.tn(v.ty)
                            ),
                        ));
                    }
                }
                // Unowned generator-valued locals could escape a close path.
                if v.ty == Type::GeneratorInt {
                    return Err(Diagnostic::new(*pos, "E_GENERATOR_OWNERSHIP",
                        "generator handles must be consumed by for-in until owned bindings support close"));
                }
                let slot = self.declare(name, v.ty, *kind == BindKind::Let, *pos)?;
                TStmt::Set(slot, v)
            }
            Stmt::Assign { name, value, pos } => {
                let (slot, ty, mutable, mail, borrow, retired) = match self.lookup(name) {
                    Some(l) => (l.slot, l.ty, l.mutable, l.mail.is_some(), l.borrow, l.borrow_retired),
                    None => return Err(Diagnostic::new(*pos, "E_UNBOUND", format!("unknown name '{name}'"))),
                };
                if let Some(kind) = borrow {
                    // Defense-in-depth: never permit alias assignment after
                    // non-lexical retirement, even if a future scanner change
                    // mistakenly omitted a write from the use inventory.
                    if retired {
                        return Err(Diagnostic::new(*pos, "E_BORROW",
                            "cannot assign through a retired scalar loan alias"));
                    }
                    if kind != BorrowKind::Exclusive {
                        return Err(Diagnostic::new(*pos, "E_BORROW",
                            "cannot write through a shared borrow"));
                    }
                    let v = self.consume_expected(value, ty)?;
                    if v.ty != ty {
                        return Err(type_err(value.pos,
                            format!("cannot assign {} through borrow of {}", self.tn(v.ty), self.tn(ty))));
                    }
                    // Assignment changes the referent, NOT the alias binding.
                    self.mark_scalar_alias_used(name, true);
                    return Ok(TStmt::Set(slot, v));
                }
                self.ensure_unloaned(slot, *pos)?;
                if !mutable || mail {
                    return Err(Diagnostic::new(
                        *pos,
                        "E_IMMUTABLE",
                        format!("cannot reassign val binding '{name}'"),
                    ));
                }
                let v = self.consume_expected(value, ty)?;
                if v.ty != ty {
                    return Err(type_err(
                        value.pos,
                        format!("cannot assign {} to '{name}' of type {}", self.tn(v.ty), self.tn(ty)),
                    ));
                }
                // a reassigned object binding owns a value again
                self.moved.remove(&slot);
                TStmt::Set(slot, v)
            }
            Stmt::FieldAssign { target, field, value, pos } => self.field_assign(target, field, value, *pos)?,
            Stmt::Expr(e) => {
                let v = self.expr(e)?;
                if v.ty == Type::GeneratorInt {
                    return Err(Diagnostic::new(e.pos, "E_GENERATOR_OWNERSHIP",
                        "discarding an unclosed generator handle is prohibited"));
                }
                TStmt::Expr(v)
            }
            Stmt::Throw(error, pos) => self.guest_throw(error, *pos)?,
            Stmt::Yield(_, pos) | Stmt::YieldFrom(_, pos) => {
                return Err(Diagnostic::new(*pos, "E_YIELD",
                    "yield and yield* require a compiled generator continuation"));
            }
            Stmt::ForIn { name, source, body, pos } => {
                let gen = self.consume(source)?;
                if gen.ty != Type::GeneratorInt {
                    return Err(type_err(source.pos,
                        format!("for-in expects Generator<int>, found {}", self.tn(gen.ty))));
                }
                if !generator_forin_safe_body(body) {
                    return Err(Diagnostic::new(*pos, "E_UNSUPPORTED",
                        "generator for-in body has effects requiring exception/cancellation-safe unwinding"));
                }
                // This first slice owns a fresh handle produced by a direct
                // generator call. No aliasing or escaping local handles.
                if !matches!(gen.e, TExpr::Call(_, _)) {
                    return Err(Diagnostic::new(source.pos, "E_GENERATOR_OWNERSHIP",
                        "for-in currently requires a fresh generator call"));
                }
                let handle_slot = self.temp(Type::GeneratorInt, *pos)?;
                self.scopes.push(Vec::new());
                let checked = (|| {
                    let value_slot = self.declare(name, Type::Int, false, *pos)?;
                    let body = self.looped(body)?;
                    Ok(TStmt::ForIn { source: gen, handle_slot, value_slot, body })
                })();
                self.scopes.pop();
                checked?
            }
            Stmt::Return(v, pos) => match (v, self.ret) {
                (None, Type::Void) => TStmt::Return(None),
                (None, t) => {
                    return Err(Diagnostic::new(
                        *pos,
                        "E_RETURN",
                        format!("return needs a value of type {}", self.tn(t)),
                    ))
                }
                (Some(e), Type::Void) => return Err(type_err(e.pos, "a void fnc cannot return a value")),
                (Some(e), t) => {
                    let v = self.consume_expected(e, t)?;
                    if v.ty != t {
                        return Err(type_err(
                            e.pos,
                            format!("return value has type {} but expected {}", self.tn(v.ty), self.tn(t)),
                        ));
                    }
                    TStmt::Return(Some(v))
                }
            },
            Stmt::If { arms, otherwise, .. } => {
                // Ownership after the if is the union over arms that can
                // fall through; arms that always return do not contribute.
                let mut out = Vec::new();
                let mut after: BTreeSet<u32> = BTreeSet::new();
                for (c, b) in arms {
                    let c = self.cond(c, "if")?;
                    let at_cond = self.moved.clone();
                    let body = self.scoped(b)?;
                    if !returns(&body) {
                        after.extend(self.moved.iter().copied());
                    }
                    self.moved = at_cond;
                    out.push((c, body));
                }
                let other = match otherwise {
                    Some(b) => {
                        let body = self.scoped(b)?;
                        if !returns(&body) {
                            after.extend(self.moved.iter().copied());
                        }
                        Some(body)
                    }
                    None => {
                        after.extend(self.moved.iter().copied());
                        None
                    }
                };
                self.moved = after;
                TStmt::If(out, other)
            }
            Stmt::While { cond, body, .. } => {
                let c = self.cond(cond, "while")?;
                let body = self.looped(body)?;
                TStmt::Loop {
                    cond: Some(c),
                    body,
                    update: Vec::new(),
                }
            }
            Stmt::Loop { body, .. } => {
                let body = self.looped(body)?;
                TStmt::Loop {
                    cond: None,
                    body,
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
                    let b = self.looped(body)?;
                    let u = match update {
                        Some(u) => vec![self.stmt(u)?],
                        None => Vec::new(),
                    };
                    out.push(TStmt::Loop {
                        cond: c,
                        body: b,
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
            Stmt::Match { subject, arms, pos } => self.match_stmt(subject, arms, *pos)?,
        })
    }

    /// `match`: the subject is evaluated once into a temp; arms are tried in
    /// source order; a guard runs only after its pattern matched. The checker
    /// rejects unreachable arms and requires exhaustiveness (Java reference:
    /// "unreachable match arm", "non-exhaustive match").
    fn match_stmt(&mut self, subject: &Expr, arms: &[MatchArm], pos: Pos) -> R<TStmt> {
        let subj = self.value(subject)?;
        let sty = subj.ty;
        if matches!(sty, Type::OptionInt | Type::ResultIntStr | Type::OptionResultIntStr) {
            return self.match_adt(subj, arms, pos);
        }
        if !matches!(sty, Type::Int | Type::Bool | Type::Enum(_) | Type::Float | Type::Str) {
            return Err(Diagnostic::new(
                subject.pos,
                "E_UNSUPPORTED",
                format!("matching a {} value is not supported by the browser compiler yet", self.tn(sty)),
            ));
        }
        let tmp = self.temp(sty, pos)?;
        let get = || TE {
            e: TExpr::Local(tmp),
            ty: sty,
        };
        let variants = match sty {
            Type::Enum(e) => self.g.enums[e as usize].variants.len(),
            Type::Bool => 2,
            _ => usize::MAX,
        };
        let mut seen: BTreeSet<i64> = BTreeSet::new();
        let mut covered = false;
        let mut out = Vec::new();
        let mut after: BTreeSet<u32> = BTreeSet::new();
        for (i, arm) in arms.iter().enumerate() {
            if covered {
                return Err(Diagnostic::new(
                    arm.pos,
                    "E_MATCH",
                    format!("unreachable match arm {}: earlier arms already match every value", i + 1),
                ));
            }
            // (pattern test, literal key for reachability, binding name)
            let (test, key, bind): (Option<TE>, Option<i64>, Option<&str>) = match &arm.pattern {
                Pattern::Wild => (None, None, None),
                Pattern::Bind(n) => (None, None, Some(n.as_str())),
                Pattern::Int(v) => {
                    if sty != Type::Int {
                        return Err(type_err(arm.pos, format!("int pattern cannot match a {} value", self.tn(sty))));
                    }
                    let lit = TE {
                        e: TExpr::I64(*v),
                        ty: Type::Int,
                    };
                    (
                        Some(TE {
                            e: TExpr::Bin(BinOp::Eq, Box::new(get()), Box::new(lit)),
                            ty: Type::Bool,
                        }),
                        Some(*v),
                        None,
                    )
                }
                Pattern::Bool(b) => {
                    if sty != Type::Bool {
                        return Err(type_err(arm.pos, format!("bool pattern cannot match a {} value", self.tn(sty))));
                    }
                    let lit = TE {
                        e: TExpr::Bool(*b),
                        ty: Type::Bool,
                    };
                    (
                        Some(TE {
                            e: TExpr::Bin(BinOp::Eq, Box::new(get()), Box::new(lit)),
                            ty: Type::Bool,
                        }),
                        Some(*b as i64),
                        None,
                    )
                }
                Pattern::Adt(name, _) => return Err(type_err(arm.pos,
                    format!("ADT pattern '{name}' cannot match a {}", self.tn(sty)))),
                Pattern::Variant(en, var) => {
                    let ev = self.enum_value(en, var, arm.pos, arm.pos)?;
                    if ev.ty != sty {
                        return Err(type_err(
                            arm.pos,
                            format!(
                                "pattern {en}.{var} has type {} but the subject has type {}",
                                self.tn(ev.ty),
                                self.tn(sty)
                            ),
                        ));
                    }
                    let ord = match ev.e {
                        TExpr::EnumVal(o) => o as i64,
                        _ => unreachable!("enum_value yields an ordinal"),
                    };
                    (
                        Some(TE {
                            e: TExpr::Bin(BinOp::Eq, Box::new(get()), Box::new(ev)),
                            ty: Type::Bool,
                        }),
                        Some(ord),
                        None,
                    )
                }
            };
            if let Some(k) = key {
                if seen.contains(&k) {
                    return Err(Diagnostic::new(
                        arm.pos,
                        "E_MATCH",
                        format!(
                            "unreachable match arm {}: an earlier arm already matches every value accepted by it",
                            i + 1
                        ),
                    ));
                }
            }
            self.scopes.push(Vec::new());
            let r = (|| -> R<(TE, Vec<TStmt>)> {
                let mut body = Vec::new();
                if let Some(n) = bind {
                    let slot = self.declare(n, sty, false, arm.pos)?;
                    body.push(TStmt::Set(slot, get()));
                }
                let guard = match &arm.guard {
                    Some(g) => Some(self.cond(g, "match guard")?),
                    None => None,
                };
                let at_arm = self.moved.clone();
                body.extend(self.stmts(&arm.body)?);
                if !returns(&body) {
                    after.extend(self.moved.iter().copied());
                }
                self.moved = at_arm;
                let truth = TE {
                    e: TExpr::Bool(true),
                    ty: Type::Bool,
                };
                let cond = match (test, guard) {
                    (None, None) => truth,
                    (Some(t), None) => t,
                    (None, Some(g)) => g,
                    // the guard must not run when the pattern fails
                    (Some(t), Some(g)) => TE {
                        e: TExpr::Ternary(
                            Box::new(t),
                            Box::new(g),
                            Box::new(TE {
                                e: TExpr::Bool(false),
                                ty: Type::Bool,
                            }),
                        ),
                        ty: Type::Bool,
                    },
                };
                Ok((cond, body))
            })();
            self.scopes.pop();
            let (cond, body) = r?;
            if arm.guard.is_none() {
                match key {
                    Some(k) => {
                        seen.insert(k);
                        if seen.len() == variants {
                            covered = true;
                        }
                    }
                    None => covered = true,
                }
            }
            out.push((cond, body));
        }
        if !covered {
            return Err(Diagnostic::new(
                pos,
                "E_MATCH",
                format!(
                    "non-exhaustive match for {}; add an unguarded 'on _ -> {{ ... }}' arm or cover every provable case",
                    self.tn(sty)
                ),
            ));
        }
        self.moved = after;
        Ok(TStmt::Block(vec![
            TStmt::Set(tmp, subj),
            TStmt::If(out, Some(vec![TStmt::Unreachable])),
        ]))
    }


    /// Check a concrete tagged ADT match. Only v1 profiles are admitted:
    /// Option<int> and Result<int,string>. Each variant has a unique tag;
    /// the payload read occurs only inside its matching guarded arm.
    fn match_adt(&mut self, subj: TE, arms: &[MatchArm], pos: Pos) -> R<TStmt> {
        let sty = subj.ty;
        let tmp = self.temp(sty, pos)?;
        let get = || TE { e: TExpr::Local(tmp), ty: sty };
        let mut seen: BTreeSet<i64> = BTreeSet::new();
        let mut covered = false;
        let mut out = Vec::new();
        let mut after: BTreeSet<u32> = BTreeSet::new();
        for (i, arm) in arms.iter().enumerate() {
            if covered {
                return Err(Diagnostic::new(arm.pos, "E_MATCH",
                    format!("unreachable match arm {}: prior arms cover every variant", i + 1)));
            }
            let (key, payload_name, payload_ty) = match &arm.pattern {
                Pattern::Wild => (None, None, None),
                Pattern::Bind(n) => (None, Some(n.as_str()), Some(sty)),
                Pattern::Adt(name, bind) => {
                    let (tag, needed) = match (sty, name.as_str()) {
                        (Type::ResultIntStr, "Ok") => (0, Some(Type::Int)),
                        (Type::ResultIntStr, "Err") => (1, Some(Type::Str)),
                        (Type::OptionInt, "None") => (0, None),
                        (Type::OptionInt, "Some") => (1, Some(Type::Int)),
                        (Type::OptionResultIntStr, "None") => (0, None),
                        (Type::OptionResultIntStr, "Some") => (1, Some(Type::ResultIntStr)),
                        _ => return Err(type_err(arm.pos,
                            format!("variant '{name}' is not part of {}", self.tn(sty)))),
                    };
                    if bind.is_some() != needed.is_some() {
                        return Err(Diagnostic::new(arm.pos, "E_ARITY",
                            format!("pattern '{name}' has incorrect payload arity")));
                    }
                    (Some(tag), bind.as_deref(), needed)
                }
                _ => return Err(type_err(arm.pos,
                    format!("pattern is not a variant of {}", self.tn(sty)))),
            };
            if key.is_some_and(|tag| seen.contains(&tag)) {
                return Err(Diagnostic::new(arm.pos, "E_MATCH",
                    format!("unreachable match arm {}: variant already covered", i + 1)));
            }
            let test = key.map(|tag| TE {
                e: TExpr::AdtIs { obj: Box::new(get()), tag: tag as u32 },
                ty: Type::Bool,
            });
            self.scopes.push(Vec::new());
            let r = (|| -> R<(TE, Vec<TStmt>)> {
                let mut body = Vec::new();
                if let Some(name) = payload_name {
                    let ty = payload_ty.expect("the pattern binds a value");
                    let slot = self.declare(name, ty, false, arm.pos)?;
                    let value = if key.is_none() {
                        get()
                    } else {
                        TE { e: TExpr::AdtPayload(Box::new(get())), ty }
                    };
                    body.push(TStmt::Set(slot, value));
                }
                let guard = match &arm.guard {
                    Some(g) => Some(self.cond(g, "match guard")?),
                    None => None,
                };
                let at_arm = self.moved.clone();
                body.extend(self.stmts(&arm.body)?);
                if !returns(&body) {
                    after.extend(self.moved.iter().copied());
                }
                self.moved = at_arm;
                let truth = TE { e: TExpr::Bool(true), ty: Type::Bool };
                let cond = match (test, guard) {
                    (None, None) => truth,
                    (Some(t), None) => t,
                    (None, Some(g)) => g,
                    (Some(t), Some(g)) => TE {
                        e: TExpr::Ternary(
                            Box::new(t), Box::new(g),
                            Box::new(TE { e: TExpr::Bool(false), ty: Type::Bool }),
                        ),
                        ty: Type::Bool,
                    },
                };
                Ok((cond, body))
            })();
            self.scopes.pop();
            let (cond, body) = r?;
            if arm.guard.is_none() {
                match key {
                    Some(k) => {
                        seen.insert(k);
                        if seen.len() == 2 { covered = true; }
                    }
                    None => covered = true,
                }
            }
            out.push((cond, body));
        }
        if !covered {
            return Err(Diagnostic::new(pos, "E_MATCH",
                format!("non-exhaustive match for {}; cover both variants or use on _", self.tn(sty))));
        }
        self.moved = after;
        Ok(TStmt::Block(vec![
            TStmt::Set(tmp, subj),
            TStmt::If(out, Some(vec![TStmt::Unreachable])),
        ]))
    }

    fn field_assign(&mut self, target: &Expr, field: &str, value: &Expr, pos: Pos) -> R<TStmt> {
        if !matches!(target.kind, ExprKind::SelfRef) {
            return Err(Diagnostic::new(
                pos,
                "E_OWNERSHIP",
                "fields are assigned only through 'self' in a constructor or an actor method",
            ));
        }
        match self.owner {
            Owner::Class { c, ctor, is_static } => {
                if is_static {
                    return Err(Diagnostic::new(target.pos, "E_UNBOUND", "unknown name 'self'"));
                }
                let class = &self.g.classes[c as usize];
                let f = match class.fields.iter().find(|f| f.name == field) {
                    Some(f) => f,
                    None => {
                        return Err(Diagnostic::new(
                            pos,
                            "E_UNBOUND",
                            format!("class '{}' has no field '{field}'", class.name),
                        ))
                    }
                };
                let (offset, fty, mode) = (f.slot, f.ty, f.mode);
                if mode == FieldMode::Final {
                    return Err(Diagnostic::new(
                        pos,
                        "E_IMMUTABLE",
                        format!(
                            "field '{}.{field}' is immutable/final; cannot reassign fixed class field",
                            class.name
                        ),
                    ));
                }
                if !ctor {
                    return Err(Diagnostic::new(
                        pos,
                        "E_OWNERSHIP",
                        format!("cannot mutate field '{field}' through read-only binding 'self'"),
                    ));
                }
                let v = self.value(value)?;
                if v.ty != fty {
                    return Err(type_err(
                        value.pos,
                        format!("cannot assign {} to field '{field}' of type {}", self.tn(v.ty), self.tn(fty)),
                    ));
                }
                Ok(TStmt::FieldSet {
                    obj: TE {
                        e: TExpr::Local(0),
                        ty: Type::Class(c),
                    },
                    offset,
                    ty: fty,
                    value: v,
                })
            }
            Owner::Actor { a } => {
                let actor = &self.g.actors[a as usize];
                let f = match actor.fields.iter().find(|f| f.name == field) {
                    Some(f) => f,
                    None => {
                        return Err(Diagnostic::new(
                            pos,
                            "E_UNBOUND",
                            format!("actor '{}' has no field '{field}'", actor.name),
                        ))
                    }
                };
                let (global, fty, mode) = (f.slot, f.ty, f.mode);
                if mode != FieldMode::ActorState {
                    return Err(Diagnostic::new(
                        pos,
                        "E_IMMUTABLE",
                        format!(
                            "field '{}.{field}' is immutable/final; cannot reassign fixed class field",
                            actor.name
                        ),
                    ));
                }
                let v = self.value(value)?;
                if v.ty != fty {
                    return Err(type_err(
                        value.pos,
                        format!("cannot assign {} to field '{field}' of type {}", self.tn(v.ty), self.tn(fty)),
                    ));
                }
                Ok(TStmt::GlobalSet(global, v))
            }
            _ => Err(Diagnostic::new(target.pos, "E_UNBOUND", "unknown name 'self'")),
        }
    }

    /// Construct an ADT only where the full expected type is unambiguous.
    /// Neither Null nor a bare payload implicitly converts to an ADT.
    fn consume_expected(&mut self, e: &Expr, want: Type) -> R<TE> {
        let (variant, argument): (&str, Option<&Expr>) = match &e.kind {
            ExprKind::Call(name, args) if matches!(name.as_str(), "Ok" | "Err" | "Some") => {
                if args.len() != 1 {
                    return Err(Diagnostic::new(e.pos, "E_ARITY",
                        format!("{name} requires one payload argument, got {}", args.len())));
                }
                (name.as_str(), Some(&args[0]))
            }
            ExprKind::Var(name) if name == "None" => ("None", None),
            _ => return self.consume(e),
        };
        let (tag, payload_type) = match (want, variant) {
            (Type::ResultIntStr, "Ok") => (0, Some(Type::Int)),
            (Type::ResultIntStr, "Err") => (1, Some(Type::Str)),
            (Type::OptionInt, "None") => (0, None),
            (Type::OptionInt, "Some") => (1, Some(Type::Int)),
            (Type::OptionResultIntStr, "None") => (0, None),
            (Type::OptionResultIntStr, "Some") => (1, Some(Type::ResultIntStr)),
            _ => return Err(type_err(e.pos,
                format!("variant '{variant}' is not valid for {}", self.tn(want)))),
        };
        let payload = match (argument, payload_type) {
            (Some(arg), Some(ty)) => {
                // A nested Some(Ok(...)) or Some(Err(...)) carries the
                // expected inner Result type; constructors are contextual.
                let value = self.consume_expected(arg, ty)?;
                if value.ty != ty {
                    return Err(type_err(arg.pos,
                        format!("'{variant}' payload must be {}, found {}", self.tn(ty), self.tn(value.ty))));
                }
                Some(Box::new(value))
            }
            (None, None) => None,
            _ => unreachable!("variant arity checked above"),
        };
        let tmp = self.temp(want, e.pos)?;
        Ok(TE { e: TExpr::AdtNew { tag, payload, tmp }, ty: want })
    }

    /// An expression used as a value (not void).
    fn value(&mut self, e: &Expr) -> R<TE> {
        let v = self.expr(e)?;
        if v.ty == Type::Void {
            return Err(type_err(e.pos, "a void call has no value"));
        }
        Ok(v)
    }

    /// A value in an ownership-transferring position (binding initializer,
    /// assignment, argument, return): naming an object local moves it.
    fn consume(&mut self, e: &Expr) -> R<TE> {
        if let ExprKind::Var(name) = &e.kind {
            if self.lookup(name).is_some_and(|l| l.borrow.is_some()) {
                return Err(Diagnostic::new(e.pos, "E_ESCAPE",
                    "borrowed alias cannot be returned, passed, or rebound as an owned value; use copy! for scalar values"));
            }
        }
        let v = self.value(e)?;
        if let (ExprKind::Var(name), TExpr::Local(slot), Type::Class(_)) = (&e.kind, &v.e, v.ty) {
            let slot = *slot;
            if self.loop_floors.last().is_some_and(|floor| slot < *floor) {
                return Err(Diagnostic::new(
                    e.pos,
                    "E_MOVED",
                    format!("cannot move '{name}' inside a loop: it is declared outside the loop"),
                ));
            }
            self.ensure_unloaned(slot, e.pos)?;
            self.moved.insert(slot);
        }
        Ok(v)
    }

    // These non-inlined helpers keep the per-level stack frame of the
    // recursively invoked expression checker bounded on the browser runner.
    #[inline(never)]
    fn trap_value(&mut self, inner: &Expr, pos: Pos) -> R<TE> {
        if self.owner != Owner::TopLevel {
            return Err(Diagnostic::new(pos, "E_UNSUPPORTED",
                "caller-side trap inside class/actor requires lifecycle integration"));
        }
        let value = self.consume(inner)?;
        let option_ty = match value.ty {
            Type::Int => Type::OptionInt,
            Type::ResultIntStr => Type::OptionResultIntStr,
            _ => return Err(Diagnostic::new(pos, "E_UNSUPPORTED_TYPE",
                format!("browser trap supports only int and Result<int,string> operands, found {}", self.tn(value.ty)))),
        };
        let value_tmp = self.temp(value.ty, pos)?;
        let some_tmp = self.temp(option_ty, pos)?;
        let none_tmp = self.temp(option_ty, pos)?;
        Ok(TE {
            e: TExpr::Trap { value: Box::new(value), value_tmp, some_tmp, none_tmp },
            ty: option_ty,
        })
    }

    #[inline(never)]
    fn guest_throw(&mut self, error: &Expr, pos: Pos) -> R<TStmt> {
        if self.owner != Owner::TopLevel {
            return Err(Diagnostic::new(pos, "E_UNSUPPORTED",
                "ordinary throw in class and actor methods needs supervisor/cleanup integration"));
        }
        let args = match &error.kind {
            ExprKind::Call(name, args) if name == "Error" => args,
            _ => return Err(type_err(error.pos,
                "throw requires an Error subclass; this browser slice admits only throw Error(message)")),
        };
        if args.len() != 1 {
            return Err(Diagnostic::new(error.pos, "E_ARITY",
                "Error(message) requires exactly one string argument"));
        }
        if self.g.names.contains_key("Error") || self.g.class("Error").is_some() {
            return Err(Diagnostic::new(error.pos, "E_RESERVED",
                "the guest Error constructor cannot be shadowed"));
        }
        let msg = self.value(&args[0])?;
        if msg.ty != Type::Str {
            return Err(type_err(args[0].pos, "Error(message) requires a string message"));
        }
        Ok(TStmt::Throw(msg))
    }

    fn expr(&mut self, e: &Expr) -> R<TE> {
        match &e.kind {
            ExprKind::Unary(UnOp::Copy, inner) => self.scalar_copy(inner, e.pos),
            ExprKind::Unary(UnOp::Take, inner) => self.scalar_take(inner, e.pos),
            ExprKind::Unary(UnOp::Borrow | UnOp::BorrowMut, _) =>
                Err(Diagnostic::new(e.pos, "E_ESCAPE",
                    "borrow! references are only admitted as scoped val bindings")),
            ExprKind::Unary(UnOp::Neg, inner) if matches!(inner.kind, ExprKind::Int(None)) => Ok(TE {
                e: TExpr::I64(i64::MIN),
                ty: Type::Int,
            }),
            ExprKind::Unary(op, inner) => self.unary(*op, inner, e.pos),
            ExprKind::Trap(inner) => self.trap_value(inner, e.pos),
            ExprKind::Binary(op, l, r) => self.binary(*op, l, r, e.pos),
            ExprKind::Ternary(c, a, b) => self.ternary(c, a, b, e.pos),
            ExprKind::Propagate(inner) => {
                let value = self.consume(inner)?;
                let success_tag = match value.ty {
                    Type::ResultIntStr => 0,
                    Type::OptionInt => 1,
                    _ => return Err(type_err(e.pos,
                        format!("postfix '?' requires Option<int> or Result<int,string>, found {}", self.tn(value.ty)))),
                };
                if self.ret != value.ty {
                    return Err(type_err(e.pos,
                        format!("postfix '?' can propagate {} only from a function returning the same type; found {}", self.tn(value.ty), self.tn(self.ret))));
                }
                let tmp = self.temp(value.ty, e.pos)?;
                Ok(TE { e: TExpr::AdtPropagate {
                    source: Box::new(value), tmp, success_tag
                }, ty: Type::Int })
            }
            // receivers that are not names recurse; keep that path thin
            ExprKind::Member(base, name) if !matches!(base.kind, ExprKind::Var(_) | ExprKind::SelfRef) => {
                let obj = self.value(base)?;
                self.member_on(obj, name, e.pos)
            }
            ExprKind::MethodCall(base, name, args) if !matches!(base.kind, ExprKind::Var(_) | ExprKind::SelfRef) => {
                let recv = self.value(base)?;
                self.method_on(recv, name, args, e.pos)
            }
            _ => self.expr_cold(e),
        }
    }

    #[inline(never)]
    fn scalar_copy(&mut self, inner: &Expr, pos: Pos) -> R<TE> {
        let value = self.value(inner)?;
        if !matches!(value.ty, Type::Int | Type::Bool | Type::Float | Type::Str) {
            return Err(Diagnostic::new(pos, "E_COPY",
                "copy! requires an admitted Copy-capable scalar; it cannot duplicate owned objects"));
        }
        Ok(value)
    }

    #[inline(never)]
    fn scalar_take(&mut self, inner: &Expr, pos: Pos) -> R<TE> {
        let ExprKind::Var(name) = &inner.kind else {
            return Err(Diagnostic::new(pos, "E_MOVE",
                "take! requires a named local binding, not an expression"));
        };
        let value = self.value(inner)?;
        if self.lookup(name).is_some_and(|l| l.borrow.is_some()) {
            return Err(Diagnostic::new(pos, "E_ESCAPE", "cannot take! from a borrowed alias"));
        }
        if !matches!(value.ty, Type::Int | Type::Bool | Type::Float | Type::Str) {
            return Err(Diagnostic::new(pos, "E_UNSUPPORTED",
                "take! of owned objects requires verified destructor and transfer semantics"));
        }
        let TExpr::Local(slot) = &value.e else {
            return Err(Diagnostic::new(pos, "E_MOVE",
                format!("take! requires a local binding, found '{name}'")));
        };
        if self.loop_floors.last().is_some_and(|floor| *slot < *floor) {
            return Err(Diagnostic::new(pos, "E_MOVED",
                format!("cannot take! '{name}' inside a loop: it is declared outside the loop")));
        }
        self.ensure_unloaned(*slot, pos)?;
        self.moved.insert(*slot);
        Ok(value)
    }

    #[inline(never)]
    fn unary(&mut self, op: UnOp, inner: &Expr, pos: Pos) -> R<TE> {
        let op = &op;
        Ok({
            let v = self.value(inner)?;
            match op {
                UnOp::Neg | UnOp::Plus if v.ty == Type::Float => {
                    if *op == UnOp::Plus {
                        v
                    } else {
                        TE {
                            e: TExpr::FNeg(Box::new(v)),
                            ty: Type::Float,
                        }
                    }
                }
                UnOp::Neg | UnOp::Plus => {
                    if v.ty != Type::Int {
                        return Err(type_err(
                            pos,
                            format!(
                                "unary '{}' needs an int or float operand, found {}",
                                if *op == UnOp::Neg { "-" } else { "+" },
                                self.tn(v.ty)
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
                UnOp::Copy | UnOp::Take | UnOp::Borrow | UnOp::BorrowMut => {
                    unreachable!("explicit ownership handled before generic unary lowering")
                }
                UnOp::Not => {
                    if v.ty != Type::Bool {
                        return Err(type_err(pos, format!("'!' needs a bool operand, found {}", self.tn(v.ty))));
                    }
                    TE {
                        e: TExpr::Not(Box::new(v)),
                        ty: Type::Bool,
                    }
                }
            }
        })
    }

    #[inline(never)]
    fn binary(&mut self, op: BinOp, l: &Expr, r: &Expr, pos: Pos) -> R<TE> {
        let op = &op;
        Ok({
            let a = self.value(l)?;
            let b = self.value(r)?;
            if (a.ty == Type::Str || b.ty == Type::Str) && !matches!(op, BinOp::Eq | BinOp::Ne) {
                let both = a.ty == Type::Str && b.ty == Type::Str;
                return match op {
                    // Java: '+' joins two strings; it never converts other operands
                    BinOp::Add if both => Ok(TE {
                        e: TExpr::Str(StrOp::Concat, Box::new(a), Some(Box::new(b))),
                        ty: Type::Str,
                    }),
                    // Java String.compareTo order
                    BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge if both => Ok(TE {
                        e: TExpr::Bin(
                            *op,
                            Box::new(TE {
                                e: TExpr::Str(StrOp::Cmp, Box::new(a), Some(Box::new(b))),
                                ty: Type::Int,
                            }),
                            Box::new(TE {
                                e: TExpr::I64(0),
                                ty: Type::Int,
                            }),
                        ),
                        ty: Type::Bool,
                    }),
                    _ => Err(type_err(
                        pos,
                        format!(
                            "operator '{}' needs numeric operands (or two strings for '+' and comparisons), found {} and {}",
                            op.symbol(),
                            self.tn(a.ty),
                            self.tn(b.ty)
                        ),
                    )),
                };
            }
            let numeric = |t: Type| matches!(t, Type::Int | Type::Float);
            if matches!(
                op,
                BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge
            ) {
                if !numeric(a.ty) || !numeric(b.ty) {
                    return Err(type_err(
                        pos,
                        format!(
                            "operator '{}' needs int or float operands, found {} and {}",
                            op.symbol(),
                            self.tn(a.ty),
                            self.tn(b.ty)
                        ),
                    ));
                }
                if a.ty == Type::Int && b.ty == Type::Int {
                    if *op == BinOp::Div {
                        return Err(Diagnostic::new(
                            pos,
                            "E_UNSUPPORTED",
                            "'/' on two ints is not admitted: whether int / int yields float or int is undecided (spec S-25); write a float operand, e.g. 7.0 / 2",
                        ));
                    }
                } else {
                    // Java binary numeric promotion: an int operand widens to float.
                    let widen = |v: TE| -> TE {
                        if v.ty == Type::Int {
                            TE {
                                e: TExpr::IntToFloat(Box::new(v)),
                                ty: Type::Float,
                            }
                        } else {
                            v
                        }
                    };
                    let ty = if matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div) {
                        Type::Float
                    } else {
                        Type::Bool
                    };
                    return Ok(TE {
                        e: TExpr::Bin(*op, Box::new(widen(a)), Box::new(widen(b))),
                        ty,
                    });
                }
            }
            let ty = match op {
                BinOp::Add | BinOp::Sub | BinOp::Mul => Type::Int,
                BinOp::Div => unreachable!("int / int rejected above"),
                BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => Type::Bool,
                BinOp::Eq | BinOp::Ne => {
                    if a.ty != b.ty {
                        return Err(type_err(
                            pos,
                            format!(
                                "'{}' operands must have the same type, found {} and {}",
                                op.symbol(),
                                self.tn(a.ty),
                                self.tn(b.ty)
                            ),
                        ));
                    }
                    if matches!(a.ty, Type::Class(_) | Type::ActorRef(_) | Type::OptionInt | Type::ResultIntStr | Type::OptionResultIntStr) {
                        return Err(Diagnostic::new(
                            pos,
                            "E_UNSUPPORTED",
                            "comparing object, actor or tagged ADT references is unsupported; pointer equality is not structural equality",
                        ));
                    }
                    if a.ty == Type::Str {
                        // strings compare by content (spec 17), never by reference
                        let eq = TE {
                            e: TExpr::Str(StrOp::Eq, Box::new(a), Some(Box::new(b))),
                            ty: Type::Bool,
                        };
                        return Ok(if *op == BinOp::Ne {
                            TE {
                                e: TExpr::Not(Box::new(eq)),
                                ty: Type::Bool,
                            }
                        } else {
                            eq
                        });
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
        })
    }

    #[inline(never)]
    fn ternary(&mut self, c: &Expr, a: &Expr, b: &Expr, pos: Pos) -> R<TE> {
        Ok({
            let c = self.cond(c, "ternary")?;
            let a = self.value(a)?;
            let b = self.value(b)?;
            if a.ty != b.ty {
                return Err(type_err(
                    pos,
                    format!(
                        "ternary branches must have the same type, found {} and {}",
                        self.tn(a.ty),
                        self.tn(b.ty)
                    ),
                ));
            }
            if matches!(a.ty, Type::Class(_)) {
                return Err(Diagnostic::new(
                    pos,
                    "E_UNSUPPORTED",
                    "a ternary choosing between objects is not supported by the browser compiler yet",
                ));
            }
            let ty = a.ty;
            TE {
                e: TExpr::Ternary(Box::new(c), Box::new(a), Box::new(b)),
                ty,
            }
        })
    }

    /// Non-recursive and rarely deep expression forms, kept out of line so
    /// the recursive path through `expr` has a small stack frame (see
    /// parser::MAX_TREE_HEIGHT and examples/stackprobe.rs).
    #[inline(never)]
    fn expr_cold(&mut self, e: &Expr) -> R<TE> {
        let pos = e.pos;
        if let Owner::FieldInit { actor } = self.owner {
            if let ExprKind::Member(base, variant) = &e.kind {
                if let ExprKind::Var(n) = &base.kind {
                    if actor {
                        return Err(Diagnostic::new(
                            pos,
                            "E_UNSUPPORTED",
                            "actor field initializers must be int or bool constants",
                        ));
                    }
                    return self.enum_value(n, variant, base.pos, pos);
                }
            }
        }
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
            ExprKind::Float(bits) => TE {
                e: TExpr::F64(f64::from_bits(*bits)),
                ty: Type::Float,
            },
            ExprKind::Str(text) => {
                let (offset, len) = self.string(text, pos)?;
                TE {
                    e: TExpr::StrConst(offset, len),
                    ty: Type::Str,
                }
            }
            ExprKind::Var(name) if name == "None" => {
                return Err(type_err(pos, "None requires a contextual Option<int> type"))
            }
            ExprKind::Var(name) => match self.lookup(name) {
                Some(l) if l.mail.is_some() => {
                    return Err(Diagnostic::new(
                        pos,
                        "E_UNSUPPORTED",
                        format!("the ActorMail envelope '{name}' as a value; read {name}.value or {name}.sequence"),
                    ))
                }
                Some(l) => {
                    if l.borrow.is_some() && l.borrow_retired {
                        return Err(Diagnostic::new(pos, "E_BORROW",
                            "scalar loan alias was used after its compiler-proven last use"));
                    }
                    if self.moved.contains(&l.slot) {
                        return Err(Diagnostic::new(pos, "E_MOVED", format!("use of moved value '{name}'")));
                    }
                    if l.borrow.is_none() && self.loan_state(l.slot).1 {
                        return Err(Diagnostic::new(pos, "E_BORROW",
                            format!("direct access to '{name}' is forbidden while its exclusive loan is live")));
                    }
                    let (slot, ty, loan_id) = (l.slot, l.ty, l.loan_id);
                    if loan_id.is_some() {
                        self.mark_scalar_alias_used(name, false);
                    }
                    TE {
                        e: match loan_id {
                            Some(loan_id) => TExpr::LoanRead { loan_id, owner_slot: slot },
                            None => TExpr::Local(slot),
                        },
                        ty,
                    }
                }
                None if self.g.names.contains_key(name) => {
                    return Err(Diagnostic::new(
                        pos,
                        "E_UNSUPPORTED",
                        "functions as values are not supported by the browser compiler yet",
                    ))
                }
                None if self.g.class(name).is_some() || self.g.enum_(name).is_some() || self.g.actor(name).is_some() => {
                    return Err(Diagnostic::new(
                        pos,
                        "E_UNSUPPORTED",
                        format!("the type name '{name}' is not a value"),
                    ))
                }
                None => return Err(Diagnostic::new(pos, "E_UNBOUND", format!("unknown name '{name}'"))),
            },
            ExprKind::Unary(..) | ExprKind::Trap(..) | ExprKind::Binary(..) | ExprKind::Ternary(..)
            | ExprKind::Propagate(..) => unreachable!("handled by expr"),
            ExprKind::Call(name, _) if matches!(name.as_str(), "Ok" | "Err" | "Some" | "None") =>
                return Err(type_err(pos, format!("'{name}' requires a contextual Result/Option type"))),
            ExprKind::Call(name, args) => self.call(name, args, pos)?,
            ExprKind::SelfRef => {
                return Err(match self.owner {
                    Owner::Class { is_static: false, .. } | Owner::Actor { .. } => Diagnostic::new(
                        pos,
                        "E_UNSUPPORTED",
                        "'self' as a value is not supported by the browser compiler yet; use self.field or self.method()",
                    ),
                    _ => Diagnostic::new(pos, "E_UNBOUND", "unknown name 'self'"),
                })
            }
            ExprKind::New(class, args) => self.new_object(class, args, pos)?,
            ExprKind::Spawn(actor) => {
                if let Some(me) = self.actor_name() {
                    return Err(Diagnostic::new(
                        pos,
                        "E_UNSUPPORTED",
                        format!("spawning from inside actor '{me}' is not supported by the browser compiler yet"),
                    ));
                }
                if matches!(self.owner, Owner::FieldInit { .. }) {
                    return Err(Diagnostic::new(pos, "E_UNSUPPORTED", "spawn in a field initializer"));
                }
                let a = match self.g.actor(actor) {
                    Some(a) => a,
                    None if self.g.class(actor).is_some() => {
                        return Err(type_err(
                            pos,
                            format!("'{actor}' is a class; create instances with new {actor}(...)"),
                        ))
                    }
                    None => return Err(Diagnostic::new(pos, "E_UNBOUND", format!("unknown actor '{actor}'"))),
                };
                self.imports.push(Import::ActorSpawn);
                TE {
                    e: TExpr::Spawn(a),
                    ty: Type::ActorRef(a),
                }
            }
            ExprKind::Member(base, name) => self.member(base, name, pos)?,
            ExprKind::MethodCall(base, name, args) => self.method_call(base, name, args, pos)?,
            ExprKind::Array(_) => return Err(Diagnostic::new(pos, "E_UNSUPPORTED",
                "array literals are parsed but not yet lowered to browser Wasm")),
            ExprKind::Lambda { .. } => return Err(Diagnostic::new(pos, "E_UNSUPPORTED",
                "lambdas are parsed but closure typing, captures, and browser Wasm lowering are not implemented")),
        })
    }

    fn enum_value(&mut self, enum_name: &str, variant: &str, base_pos: Pos, pos: Pos) -> R<TE> {
        let ei = match self.g.enum_(enum_name) {
            Some(e) => e,
            None => {
                return Err(Diagnostic::new(
                    base_pos,
                    "E_UNSUPPORTED",
                    format!("field initializers may only read enum variants; '{enum_name}' is not an enum"),
                ))
            }
        };
        let decl = &self.g.enums[ei as usize];
        match decl.variants.iter().position(|(v, _)| v == variant) {
            Some(i) => Ok(TE {
                e: TExpr::EnumVal(i as u32),
                ty: Type::Enum(ei),
            }),
            None => Err(Diagnostic::new(
                pos,
                "E_UNBOUND",
                format!("enum '{}' has no variant '{variant}'", decl.name),
            )),
        }
    }

    fn new_object(&mut self, class: &str, args: &[Expr], pos: Pos) -> R<TE> {
        if let Some(me) = self.actor_name() {
            return Err(isolation(pos, me, "sibling class", class));
        }
        if matches!(self.owner, Owner::FieldInit { .. }) {
            return Err(Diagnostic::new(pos, "E_UNSUPPORTED", "'new' in a field initializer"));
        }
        let ci = match self.g.class(class) {
            Some(c) => c,
            None if self.g.actor(class).is_some() => {
                return Err(type_err(pos, format!("'{class}' is an actor; start it with spawn {class}()")))
            }
            None => return Err(Diagnostic::new(pos, "E_UNBOUND", format!("unknown class '{class}'"))),
        };
        let info = &self.g.classes[ci as usize];
        if args.len() != info.ctor_params.len() {
            return Err(Diagnostic::new(
                pos,
                "E_ARITY",
                format!(
                    "constructor for {class} expects {} argument(s), got {}",
                    info.ctor_params.len(),
                    args.len()
                ),
            ));
        }
        let (idx, params) = (info.new_idx, info.ctor_params.clone());
        let targs = self.args(&format!("{class} constructor"), args, &params)?;
        Ok(TE {
            e: TExpr::Call(idx, targs),
            ty: Type::Class(ci),
        })
    }

    /// Check arguments against parameter types; object arguments move.
    fn args(&mut self, callee: &str, args: &[Expr], params: &[Type]) -> R<Vec<TE>> {
        let mut out = Vec::new();
        for (i, (a, want)) in args.iter().zip(params).enumerate() {
            let v = self.consume_expected(a, *want)?;
            if v.ty != *want {
                return Err(type_err(
                    a.pos,
                    format!(
                        "argument {} of '{callee}' has type {} but expected {}",
                        i + 1,
                        self.tn(v.ty),
                        self.tn(*want)
                    ),
                ));
            }
            out.push(v);
        }
        Ok(out)
    }

    fn arity(&self, callee: &str, want: usize, got: usize, pos: Pos) -> R<()> {
        if want != got {
            return Err(Diagnostic::new(
                pos,
                "E_ARITY",
                format!("call arity mismatch: '{callee}' takes {want} argument(s), found {got}"),
            ));
        }
        Ok(())
    }

    /// The class whose private members the current code may use.
    fn inside_class(&self) -> Option<u32> {
        match self.owner {
            Owner::Class { c, .. } => Some(c),
            _ => None,
        }
    }

    fn member(&mut self, base: &Expr, name: &str, pos: Pos) -> R<TE> {
        if let ExprKind::Var(n) = &base.kind {
            if let Some(l) = self.lookup(n) {
                if let Some(payload) = l.mail {
                    return match name {
                        "value" if payload == Type::Bool => Ok(TE {
                            e: TExpr::Bin(
                                BinOp::Ne,
                                Box::new(TE {
                                    e: TExpr::Local(0),
                                    ty: Type::Int,
                                }),
                                Box::new(TE {
                                    e: TExpr::I64(0),
                                    ty: Type::Int,
                                }),
                            ),
                            ty: Type::Bool,
                        }),
                        "value" => Ok(TE {
                            e: TExpr::Local(0),
                            ty: Type::Int,
                        }),
                        "sequence" => Ok(TE {
                            e: TExpr::Local(1),
                            ty: Type::Int,
                        }),
                        "recipient" | "sender" => Err(Diagnostic::new(
                            pos,
                            "E_UNSUPPORTED_TYPE",
                            format!("ActorMail.{name} (ActorId values) is not supported by the browser compiler yet"),
                        )),
                        _ => Err(Diagnostic::new(pos, "E_UNBOUND", format!("ActorMail has no member '{name}'"))),
                    };
                }
            } else if self.g.enum_(n).is_some() {
                if let Some(me) = self.actor_name() {
                    return Err(isolation(base.pos, me, "sibling enum", n));
                }
                return self.enum_value(n, name, base.pos, pos);
            } else if self.g.class(n).is_some() {
                return Err(Diagnostic::new(
                    pos,
                    "E_UNSUPPORTED",
                    "static fields are not supported by the browser compiler yet",
                ));
            }
        }
        if let ExprKind::SelfRef = base.kind {
            match self.owner {
                Owner::Class { c, is_static: false, .. } => {
                    return self.field_get(
                        TE {
                            e: TExpr::Local(0),
                            ty: Type::Class(c),
                        },
                        name,
                        pos,
                    )
                }
                Owner::Actor { a } => {
                    let actor = &self.g.actors[a as usize];
                    return match actor.fields.iter().find(|f| f.name == name) {
                        Some(f) => Ok(TE {
                            e: TExpr::GlobalGet(f.slot),
                            ty: f.ty,
                        }),
                        None => Err(Diagnostic::new(
                            pos,
                            "E_UNBOUND",
                            format!("actor '{}' has no field '{name}'", actor.name),
                        )),
                    };
                }
                _ => return Err(Diagnostic::new(base.pos, "E_UNBOUND", "unknown name 'self'")),
            }
        }
        let obj = self.value(base)?;
        self.member_on(obj, name, pos)
    }

    #[inline(never)]
    fn member_on(&mut self, obj: TE, name: &str, pos: Pos) -> R<TE> {
        if obj.ty == Type::Str {
            return Err(string_member_err(name, pos));
        }
        match obj.ty {
            Type::Class(_) => self.field_get(obj, name, pos),
            Type::ActorRef(_) => Err(Diagnostic::new(
                pos,
                "E_PRIVATE",
                "actor fields are private mailbox-owned state; communicate with send(...)",
            )),
            t => Err(type_err(pos, format!("{} has no member '{name}'", self.tn(t)))),
        }
    }

    fn field_get(&mut self, obj: TE, name: &str, pos: Pos) -> R<TE> {
        let Type::Class(c) = obj.ty else {
            unreachable!("field_get on a class")
        };
        let class = &self.g.classes[c as usize];
        let f = match class.fields.iter().find(|f| f.name == name) {
            Some(f) => f,
            None => {
                let what = if class.methods.iter().any(|m| m.name == name) {
                    format!("'{}.{name}' is a method; call it with ()", class.name)
                } else {
                    format!("class '{}' has no field '{name}'", class.name)
                };
                return Err(Diagnostic::new(pos, "E_UNBOUND", what));
            }
        };
        if !f.public && self.inside_class() != Some(c) {
            return Err(Diagnostic::new(
                pos,
                "E_PRIVATE",
                format!(
                    "private field '{}.{name}' is accessible only from code declared in class {}",
                    class.name, class.name
                ),
            ));
        }
        Ok(TE {
            e: TExpr::FieldGet {
                obj: Box::new(obj),
                offset: f.slot,
            },
            ty: f.ty,
        })
    }

    fn method_call(&mut self, base: &Expr, name: &str, args: &[Expr], pos: Pos) -> R<TE> {
        // Class.static(args)
        if let ExprKind::Var(n) = &base.kind {
            if self.lookup(n).is_none() {
                if let Some(c) = self.g.class(n) {
                    if let Some(me) = self.actor_name() {
                        return Err(isolation(base.pos, me, "sibling class", n));
                    }
                    let class = &self.g.classes[c as usize];
                    let m = match class.methods.iter().find(|m| m.name == name) {
                        Some(m) => m.clone(),
                        None => {
                            return Err(Diagnostic::new(
                                pos,
                                "E_UNBOUND",
                                format!("class '{n}' has no static function '{name}'"),
                            ))
                        }
                    };
                    if !m.is_static {
                        return Err(type_err(pos, format!("'{n}.{name}' is an instance method; call it on an instance")));
                    }
                    if !m.public && self.inside_class() != Some(c) {
                        return Err(Diagnostic::new(
                            pos,
                            "E_PRIVATE",
                            format!("private static function '{n}.{name}' is accessible only from code declared in class {n}"),
                        ));
                    }
                    let label = format!("{n}.{name}");
                    self.arity(&label, m.params.len(), args.len(), pos)?;
                    let targs = self.args(&label, args, &m.params)?;
                    return Ok(TE {
                        e: TExpr::Call(m.idx, targs),
                        ty: m.ret,
                    });
                }
                if self.g.enum_(n).is_some() || self.g.actor(n).is_some() {
                    return Err(Diagnostic::new(pos, "E_UNSUPPORTED", format!("'{n}' has no callable members")));
                }
            }
        }
        // self.method(args)
        if let ExprKind::SelfRef = base.kind {
            match self.owner {
                Owner::Class { c, is_static: false, .. } => {
                    let recv = TE {
                        e: TExpr::Local(0),
                        ty: Type::Class(c),
                    };
                    return self.instance_call(recv, c, name, args, pos);
                }
                Owner::Actor { a } => {
                    let actor = &self.g.actors[a as usize];
                    if name == "receive" {
                        return Err(Diagnostic::new(
                            pos,
                            "E_ACTOR",
                            "receive runs only for delivered mail; it cannot be called directly",
                        ));
                    }
                    let m = match actor.methods.iter().find(|m| m.name == name) {
                        Some(m) => m.clone(),
                        None => {
                            return Err(Diagnostic::new(
                                pos,
                                "E_UNBOUND",
                                format!("actor '{}' has no method '{name}'", actor.name),
                            ))
                        }
                    };
                    let label = format!("{}.{name}", actor.name);
                    self.arity(&label, m.params.len(), args.len(), pos)?;
                    let targs = self.args(&label, args, &m.params)?;
                    return Ok(TE {
                        e: TExpr::Call(m.idx, targs),
                        ty: m.ret,
                    });
                }
                _ => return Err(Diagnostic::new(base.pos, "E_UNBOUND", "unknown name 'self'")),
            }
        }
        let recv = self.value(base)?;
        self.method_on(recv, name, args, pos)
    }

    #[inline(never)]
    fn method_on(&mut self, recv: TE, name: &str, args: &[Expr], pos: Pos) -> R<TE> {
        if recv.ty == Type::Str {
            return self.string_method(recv, name, args, pos);
        }
        match recv.ty {
            Type::Class(c) => self.instance_call(recv, c, name, args, pos),
            Type::ActorRef(a) => {
                let actor = &self.g.actors[a as usize];
                if name != "send" {
                    return Err(Diagnostic::new(
                        pos,
                        "E_UNSUPPORTED",
                        format!(
                            "calling actor methods directly ('{}.{name}') is not supported; send a message with send(...)",
                            actor.name
                        ),
                    ));
                }
                if actor.receive_idx.is_none() {
                    return Err(type_err(
                        pos,
                        format!(
                            "actor '{}' has no receive(ActorMail<T>) handler, so it cannot accept send",
                            actor.name
                        ),
                    ));
                }
                let (payload, actor_name) = (actor.payload, actor.name.clone());
                self.arity("ActorRef.send", 1, args.len(), pos)?;
                // Mail admission transfers an owned, Send-capable scalar value.
                // A lexical borrow alias must be copied explicitly; sending
                // the alias itself must not silently escape its owner scope.
                let v = self.consume(&args[0])?;
                if v.ty != payload {
                    return Err(type_err(
                        args[0].pos,
                        format!(
                            "ActorRef.send mailbox value has type {} but '{actor_name}' receives ActorMail<{}>",
                            self.tn(v.ty),
                            self.tn(payload)
                        ),
                    ));
                }
                self.imports.push(Import::ActorSend);
                Ok(TE {
                    e: TExpr::Send {
                        target: Box::new(recv),
                        value: Box::new(v),
                    },
                    ty: Type::Void,
                })
            }
            t => Err(type_err(pos, format!("{} has no method '{name}'", self.tn(t)))),
        }
    }

    fn instance_call(&mut self, recv: TE, c: u32, name: &str, args: &[Expr], pos: Pos) -> R<TE> {
        let class = &self.g.classes[c as usize];
        let m = match class.methods.iter().find(|m| m.name == name) {
            Some(m) => m.clone(),
            None => {
                return Err(Diagnostic::new(
                    pos,
                    "E_UNBOUND",
                    format!("class '{}' has no method '{name}'", class.name),
                ))
            }
        };
        let cname = class.name.clone();
        if m.is_static {
            return Err(type_err(
                pos,
                format!("'{cname}.{name}' is a static function; call it as {cname}.{name}(...)"),
            ));
        }
        if !m.public && self.inside_class() != Some(c) {
            return Err(Diagnostic::new(
                pos,
                "E_PRIVATE",
                format!("private method '{cname}.{name}' is accessible only from code declared in class {cname}"),
            ));
        }
        let label = format!("{cname}.{name}");
        self.arity(&label, m.params.len(), args.len(), pos)?;
        let mut targs = vec![recv];
        targs.extend(self.args(&label, args, &m.params)?);
        Ok(TE {
            e: TExpr::Call(m.idx, targs),
            ty: m.ret,
        })
    }

    fn call(&mut self, name: &str, args: &[Expr], pos: Pos) -> R<TE> {
        if matches!(self.owner, Owner::FieldInit { .. }) {
            return Err(Diagnostic::new(pos, "E_UNSUPPORTED", "calls in a field initializer"));
        }
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
                let (offset, len) = self.string(s, pos)?;
                self.imports.push(Import::PrintStr);
                return Ok(TE {
                    e: TExpr::PrintStr { offset, len, newline },
                    ty: Type::Void,
                });
            }
            let v = self.value(&args[0])?;
            return match v.ty {
                Type::Float => {
                    self.imports.push(Import::PrintF64);
                    Ok(TE {
                        e: TExpr::Print {
                            value: Box::new(v),
                            newline,
                        },
                        ty: Type::Void,
                    })
                }
                Type::Str => {
                    self.imports.push(Import::PrintStr);
                    let tmp = self.temp(Type::Str, pos)?;
                    Ok(TE {
                        e: TExpr::PrintStrVal {
                            value: Box::new(v),
                            tmp,
                            newline,
                        },
                        ty: Type::Void,
                    })
                }
                Type::Int | Type::Bool => {
                    self.imports
                        .push(if v.ty == Type::Int { Import::PrintI64 } else { Import::PrintBool });
                    Ok(TE {
                        e: TExpr::Print {
                            value: Box::new(v),
                            newline,
                        },
                        ty: Type::Void,
                    })
                }
                Type::Class(c) => self.print_object(v, c, newline, pos),
                t => Err(Diagnostic::new(
                    args[0].pos,
                    "E_UNSUPPORTED",
                    format!("printing a {} value is not supported by the browser compiler yet", self.tn(t)),
                )),
            };
        }
        if self.lookup(name).is_some() {
            return Err(Diagnostic::new(
                pos,
                "E_UNSUPPORTED",
                format!("'{name}' is a local value, not a function"),
            ));
        }
        if let Some(me) = self.actor_name() {
            if self.g.names.contains_key(name) {
                return Err(isolation(pos, me, "file/module callable", name));
            }
        }
        let (idx, params, ret) = match self.g.sigs.get(&(name.to_string(), args.len())) {
            Some(s) => s.clone(),
            None => {
                return Err(match self.g.names.get(name) {
                    Some(arities) => Diagnostic::new(
                        pos,
                        "E_ARITY",
                        format!(
                            "call arity mismatch: '{name}' takes {} argument(s), found {}",
                            arities.iter().map(|a| a.to_string()).collect::<Vec<_>>().join(" or "),
                            args.len()
                        ),
                    ),
                    None if self.g.class(name).is_some() => Diagnostic::new(
                        pos,
                        "E_NEW_REQUIRED",
                        format!("'{name}' is a class; create instances with new {name}(...)"),
                    ),
                    None => Diagnostic::new(pos, "E_UNBOUND", format!("unknown name '{name}'")),
                })
            }
        };
        let targs = self.args(name, args, &params)?;
        Ok(TE {
            e: TExpr::Call(idx, targs),
            ty: ret,
        })
    }

    /// The Java reference's string members (TypeChecker string member
    /// table) that lower without allocation.
    fn string_method(&mut self, recv: TE, name: &str, args: &[Expr], pos: Pos) -> R<TE> {
        let (op, arity) = match name {
            "trim" => (StrOp::Trim, 0),
            "starts_with" => (StrOp::StartsWith, 1),
            "ends_with" => (StrOp::EndsWith, 1),
            "contains_literal" => (StrOp::Contains, 1),
            _ => return Err(string_member_err(name, pos)),
        };
        let label = format!("string.{name}");
        self.arity(&label, arity, args.len(), pos)?;
        let arg = match args.first() {
            Some(a) => {
                let v = self.value(a)?;
                if v.ty != Type::Str {
                    return Err(type_err(a.pos, format!("{label} needs a string argument, found {}", self.tn(v.ty))));
                }
                Some(Box::new(v))
            }
            None => None,
        };
        Ok(TE {
            ty: if op == StrOp::Trim { Type::Str } else { Type::Bool },
            e: TExpr::Str(op, Box::new(recv), arg),
        })
    }

    fn string(&mut self, s: &str, pos: Pos) -> R<(u32, u32)> {
        if self.data.len() + s.len() > MAX_STRING_DATA {
            return Err(Diagnostic::new(pos, "E_SOURCE_LIMIT", "too much string literal data"));
        }
        let offset = self.data.len() as u32;
        self.data.extend_from_slice(s.as_bytes());
        Ok((offset, s.len() as u32))
    }

    /// `Name{a=1, b=true}`: the Java reference's object rendering.
    fn print_object(&mut self, v: TE, c: u32, newline: bool, pos: Pos) -> R<TE> {
        let class = &self.g.classes[c as usize];
        let mut text = format!("{}{{", class.name);
        let mut plan: Vec<(String, Option<(u32, Type)>)> = Vec::new();
        for (i, f) in class.fields.iter().enumerate() {
            if let Type::Enum(_) = f.ty {
                return Err(Diagnostic::new(
                    pos,
                    "E_UNSUPPORTED",
                    format!("printing class '{}' with enum field '{}' is not supported yet", class.name, f.name),
                ));
            }
            if i > 0 {
                text.push_str(", ");
            }
            text.push_str(&f.name);
            text.push('=');
            plan.push((std::mem::take(&mut text), Some((f.slot, f.ty))));
        }
        text.push('}');
        plan.push((text, None));
        let mut pieces = Vec::new();
        for (s, field) in plan {
            let (off, len) = self.string(&s, pos)?;
            pieces.push(PrintPiece::Str(off, len));
            if let Some((offset, ty)) = field {
                self.imports.push(match ty {
                    Type::Int => Import::PrintI64,
                    Type::Float => Import::PrintF64,
                    Type::Str => Import::PrintStr,
                    _ => Import::PrintBool,
                });
                pieces.push(PrintPiece::Field(offset, ty));
            }
        }
        self.imports.push(Import::PrintStr);
        let tmp = self.temp(Type::Class(c), pos)?;
        Ok(TE {
            e: TExpr::PrintObj {
                obj: Box::new(v),
                tmp,
                pieces,
                newline,
            },
            ty: Type::Void,
        })
    }
}


/// Check proposed nominal actor interfaces *inside the production AST*.
/// This does not typecheck the concrete method bodies or enable codegen.
/// It prevents duplicate or incompatible in-actor/out-of-line declarations.
pub fn check_target_actor_interface(program: &Program) -> R<()> {
    let Some(interface) = &program.actor_interface else { return Ok(()); };
    for (i, name) in interface.forwards.iter().enumerate() {
        if interface.forwards[..i].contains(name) {
            return Err(Diagnostic::new(Pos::default(), "E_DUPLICATE",
                format!("duplicate forward actor '{name}'")));
        }
    }
    for (i, actor) in program.actors.iter().enumerate() {
        if !interface.forwards.contains(&actor.name) {
            return Err(Diagnostic::new(actor.pos, "E_UNBOUND",
                format!("actor definition '{}' has no matching declaration", actor.name)));
        }
        if program.actors[..i].iter().any(|a| a.name == actor.name) {
            return Err(Diagnostic::new(actor.pos, "E_DUPLICATE",
                format!("duplicate concrete actor '{}'", actor.name)));
        }
        let owner = interface.forwards.iter().position(|n| n == &actor.name)
            .expect("validated actor declaration");
        for method in interface.methods.iter().filter(|m| m.owner as usize == owner) {
            if method.name == "receive" {
                return Err(Diagnostic::new(method.pos, "E_SIGNATURE",
                    "receive is a sealed actor lifecycle handler"));
            }
            if actor.fields.iter().any(|f| f.name == method.name) {
                return Err(Diagnostic::new(method.pos, "E_DUPLICATE",
                    format!("receiver '{}.{}' collides with actor state", actor.name, method.name)));
            }
            for inside in &actor.methods {
                if inside.name == method.name && inside.params.len() == method.params.len() {
                    let equal = inside.ret == method.ret &&
                        inside.public == method.public &&
                        inside.params.iter().zip(&method.params).all(|(a,b)| a.ty == b.ty);
                    return Err(Diagnostic::new(method.pos,
                        if equal { "E_DUPLICATE" } else { "E_SIGNATURE" },
                        format!("receiver '{}.{}' collides with in-actor method",
                            actor.name, method.name)));
                }
            }
        }
    }
    Ok(())
}
