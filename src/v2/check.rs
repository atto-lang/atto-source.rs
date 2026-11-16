//! Type checking and lowering of the v2 syntax tree to the uniform-value IR.
//!
//! Expressions lower to an [`Ir`] tree plus statements that must run first;
//! those accumulate in the function context's `pre` buffer and are flushed in
//! front of the statement being lowered. Every suspending call is hoisted
//! this way, so suspension points only ever appear at statement level, which
//! is what the state-machine emitter requires.

use super::ast::*;
use super::ir::*;
use super::types::{fits, merge, Ty};
use crate::diag::{Diagnostic, Pos};
use std::collections::BTreeMap;

type R<T> = Result<T, Diagnostic>;

pub const MAX_LOCALS: u32 = 2048;
pub const MAX_STRING_DATA: usize = 32 * 1024;
pub const MAX_FUNCTIONS: usize = 4096;

/// panic_code values raised by lowered code (the prelude documents 6..=12).
pub const PANIC_MATCH: i32 = 13;

const TAG_INT: i64 = 1;
const TAG_BOOL: i64 = 2;
const TAG_STR: i64 = 3;

fn err(pos: Pos, code: &'static str, msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new(pos, code, msg)
}
fn type_err(pos: Pos, msg: impl Into<String>) -> Diagnostic {
    err(pos, "E_TYPE", msg)
}
fn unsupported(pos: Pos, what: &str) -> Diagnostic {
    err(pos, "E_UNSUPPORTED", format!("{what} is not supported by the browser compiler yet"))
}

#[derive(Clone)]
struct FnSig {
    params: Vec<Ty>,
    ret: Ty,
    suspends: bool,
    /// `suspends fnc f(): AsyncGenerator<T>`: called without `await`
    async_gen: Option<Ty>,
}

#[derive(Clone)]
struct FieldInfo {
    name: String,
    ty: Ty,
    mutable: bool,
    offset: u32,
}

struct ObjInfo {
    kind: ObjKind,
    name: String,
    fields: Vec<FieldInfo>,
    methods: Vec<(String, u32)>,
    /// payload type of `receive(ActorMail<T> mail)`
    mail: Ty,
    receive: Option<u32>,
    /// constructor: `spawn A()` or `S()`
    ctor: u32,
    dispose: u32,
    size: u32,
    /// type passed to `emit` inside this actor, once seen
    emit: Option<Ty>,
}

#[derive(Clone)]
struct Var {
    name: String,
    slot: u32,
    ty: Ty,
    mutable: bool,
}

struct FnCx {
    nlocals: u32,
    scopes: Vec<Vec<Var>>,
    /// flow-sensitive refinements of `Any` variables: (slot, narrowed type)
    narrow: Vec<(u32, Ty)>,
    ret: Ty,
    ret_infer: bool,
    suspends: bool,
    prelude: bool,
    is_closure: bool,
    /// (slot in the enclosing function, slot here)
    captures: Vec<(u32, u32)>,
    /// cleanup-stack height at each enclosing loop, and whether it has a break
    loops: Vec<(usize, bool)>,
    /// scope-exit actions of enclosing `using`/`with`, innermost last
    cleanups: Vec<Vec<IrStmt>>,
    pre: Vec<IrStmt>,
    /// hidden channel of an async generator body: target of `yield`
    gen_chan: Option<(u32, Ty)>,
    has_susp: bool,
}

impl FnCx {
    fn new(ret: Ty, suspends: bool, prelude: bool) -> Self {
        FnCx {
            nlocals: 0,
            scopes: vec![Vec::new()],
            narrow: Vec::new(),
            ret,
            ret_infer: false,
            suspends,
            prelude,
            is_closure: false,
            captures: Vec::new(),
            loops: Vec::new(),
            cleanups: Vec::new(),
            pre: Vec::new(),
            gen_chan: None,
            has_susp: false,
        }
    }
}

pub struct Checker<'a> {
    prog: &'a Program,
    prelude: &'a Program,
    fns: Vec<Option<IrFn>>,
    sigs: Vec<FnSig>,
    user: BTreeMap<String, u32>,
    rt_ids: BTreeMap<String, u32>,
    rt_work: Vec<(usize, u32)>,
    objs: Vec<ObjInfo>,
    cx: Vec<FnCx>,
    data: Vec<u8>,
    table: Vec<u32>,
    uses_print: [bool; 4],
    needs_sched: bool,
    has_time: bool,
    has_telemetry: bool,
}

fn add(a: Ir, b: i64) -> Ir {
    if b == 0 {
        a
    } else {
        Ir::Bin(Op::AddRaw, Box::new(a), Box::new(Ir::Const(b)))
    }
}
fn load(a: Ir, off: i64) -> Ir {
    Ir::Load(Box::new(add(a, off)))
}
fn eq(a: Ir, b: Ir) -> Ir {
    Ir::Bin(Op::Eq, Box::new(a), Box::new(b))
}
fn not(a: Ir) -> Ir {
    Ir::Not(Box::new(a))
}
fn simple(ir: &Ir) -> bool {
    matches!(ir, Ir::Const(_) | Ir::Local(_))
}

pub fn check(prog: &Program, prelude: &Program) -> R<Module> {
    let mut c = Checker {
        prog,
        prelude,
        fns: Vec::new(),
        sigs: Vec::new(),
        user: BTreeMap::new(),
        rt_ids: BTreeMap::new(),
        rt_work: Vec::new(),
        objs: Vec::new(),
        cx: Vec::new(),
        data: Vec::new(),
        table: Vec::new(),
        uses_print: [false; 4],
        needs_sched: false,
        has_time: false,
        has_telemetry: false,
    };
    c.run()
}

impl<'a> Checker<'a> {
    // ------------------------------------------------------------- driver

    fn run(&mut self) -> R<Module> {
        let prog = self.prog;
        for (path, pos) in &prog.uses {
            match path.join(".").as_str() {
                "std.time" => self.has_time = true,
                "core.telemetry" => self.has_telemetry = true,
                other => return Err(err(*pos, "E_MODULE", format!("unknown module '{other}'"))),
            }
        }
        // 1. declare object and function names
        for o in &prog.objects {
            self.reserved_name(&o.name, o.pos)?;
            if self.objs.iter().any(|x| x.name == o.name) {
                return Err(err(o.pos, "E_DUPLICATE", format!("'{}' is already defined", o.name)));
            }
            self.objs.push(ObjInfo {
                kind: o.kind,
                name: o.name.clone(),
                fields: Vec::new(),
                methods: Vec::new(),
                mail: Ty::Void,
                receive: None,
                ctor: 0,
                dispose: 0,
                size: 0,
                emit: None,
            });
        }
        for (i, o) in prog.objects.iter().enumerate() {
            let void = FnSig { params: Vec::new(), ret: Ty::Void, suspends: false, async_gen: None };
            self.objs[i].ctor = self.reserve(void.clone());
            self.objs[i].dispose = self.reserve(void.clone());
            if let Some(r) = &o.receive {
                self.objs[i].mail = self.resolve(&r.payload, false)?;
                self.objs[i].receive = Some(self.reserve(FnSig { suspends: true, ..void.clone() }));
            }
            for m in &o.methods {
                self.reserved_name(&m.name, m.pos)?;
                if self.objs[i].methods.iter().any(|(n, _)| *n == m.name) {
                    return Err(err(m.pos, "E_DUPLICATE", format!("method '{}' is already defined", m.name)));
                }
                let sig = self.signature(m, false)?;
                let id = self.reserve(sig);
                self.objs[i].methods.push((m.name.clone(), id));
            }
        }
        for f in &prog.funcs {
            self.reserved_name(&f.name, f.pos)?;
            if self.user.contains_key(&f.name) || self.objs.iter().any(|o| o.name == f.name) {
                return Err(err(f.pos, "E_DUPLICATE", format!("'{}' is already defined", f.name)));
            }
            let sig = self.signature(f, false)?;
            let id = self.reserve(sig);
            self.user.insert(f.name.clone(), id);
        }
        // 2. constructors fix the field types, then handlers and functions
        for i in 0..prog.objects.len() {
            self.lower_ctor(i)?;
        }
        for (i, o) in prog.objects.iter().enumerate() {
            if let Some(r) = &o.receive {
                self.lower_receive(i, r)?;
            }
            for (k, m) in o.methods.iter().enumerate() {
                let id = self.objs[i].methods[k].1;
                self.lower_fn(m, id, Some(i), false)?;
            }
        }
        for f in &prog.funcs {
            let id = self.user[&f.name];
            self.lower_fn(f, id, None, false)?;
        }
        let main = self.entry()?;
        // 3. the runtime functions the program reached
        while let Some((idx, id)) = self.rt_work.pop() {
            let f = &self.prelude.funcs[idx];
            self.lower_fn(f, id, None, true)?;
        }
        if self.fns.len() > MAX_FUNCTIONS {
            return Err(err(Pos { line: 1, col: 1 }, "E_SOURCE_LIMIT", "too many functions"));
        }
        let fns = self.fns.drain(..).map(|f| f.expect("every reserved function is lowered")).collect();
        Ok(Module {
            fns,
            main,
            data: std::mem::take(&mut self.data),
            table: std::mem::take(&mut self.table),
            uses_print: self.uses_print,
        })
    }

    fn reserved_name(&self, name: &str, pos: Pos) -> R<()> {
        if name.starts_with("__") {
            return Err(err(pos, "E_RESERVED", "names starting with '__' are reserved for the runtime"));
        }
        Ok(())
    }

    fn reserve(&mut self, sig: FnSig) -> u32 {
        self.fns.push(None);
        self.sigs.push(sig);
        (self.fns.len() - 1) as u32
    }

    fn signature(&self, f: &Func, prelude: bool) -> R<FnSig> {
        if f.is_gen {
            return Err(unsupported(f.pos, "'fnc *gen' generators together with async/pattern features"));
        }
        let mut params = Vec::new();
        for p in &f.params {
            let t = self.resolve(&p.ty, prelude)?;
            if t == Ty::Void {
                return Err(type_err(p.pos, "a parameter cannot have type void"));
            }
            params.push(t);
        }
        let ret = match &f.ret {
            None => Ty::Void,
            Some(t) => self.resolve(t, prelude)?,
        };
        let async_gen = match &ret {
            Ty::AsyncGen(t) => Some((**t).clone()),
            _ => None,
        };
        if async_gen.is_some() && !f.suspends {
            return Err(type_err(f.pos, "a function returning AsyncGenerator<T> is declared 'suspends'"));
        }
        Ok(FnSig { params, ret, suspends: f.suspends, async_gen })
    }

    /// The exported `main`: the user's `main` itself for a purely synchronous
    /// program, otherwise a wrapper that runs it as the first task.
    fn entry(&mut self) -> R<u32> {
        let Some(&main) = self.user.get("main") else {
            return Err(err(Pos { line: 1, col: 1 }, "E_MAIN", "program needs a 'pub fnc main(): int' entry point"));
        };
        let sig = self.sigs[main as usize].clone();
        let pos = self.prog.funcs.iter().find(|f| f.name == "main").map(|f| f.pos).unwrap();
        if !sig.params.is_empty() {
            return Err(err(pos, "E_MAIN", "main takes no parameters"));
        }
        if !matches!(sig.ret, Ty::Int | Ty::Void) {
            return Err(err(pos, "E_MAIN", "main returns int or void"));
        }
        if !sig.suspends && !self.needs_sched {
            return Ok(main);
        }
        let none = FnSig { params: vec![Ty::Raw], ret: Ty::Raw, suspends: true, async_gen: None };
        let task = self.reserve(none.clone());
        let body = if sig.suspends {
            vec![IrStmt::SCall(Some(1), main, Vec::new()), IrStmt::Return(Ir::Local(1))]
        } else {
            vec![IrStmt::Return(Ir::Call(main, Vec::new()))]
        };
        self.fns[task as usize] =
            Some(IrFn { name: "atto.main_task".into(), params: 1, locals: 2, body, suspends: sig.suspends });
        let slot = self.table.len() as i64;
        self.table.push(task);
        let start = self.reserve(FnSig { params: Vec::new(), ..none });
        let run = self.rt_id("__main");
        self.fns[start as usize] = Some(IrFn {
            name: "atto.start".into(),
            params: 0,
            locals: 1,
            body: vec![
                IrStmt::Set(0, Ir::Helper(Helper::Alloc, vec![Ir::Const(8)])),
                IrStmt::Store(Ir::Local(0), Ir::Const(slot)),
                IrStmt::Return(Ir::Call(run, vec![Ir::Local(0)])),
            ],
            suspends: false,
        });
        Ok(start)
    }

    // ------------------------------------------------------------ helpers

    fn c(&mut self) -> &mut FnCx {
        self.cx.last_mut().expect("inside a function")
    }
    fn cur(&self) -> &FnCx {
        self.cx.last().expect("inside a function")
    }
    fn temp(&mut self) -> u32 {
        let c = self.c();
        c.nlocals += 1;
        c.nlocals - 1
    }
    fn spill(&mut self, ir: Ir) -> Ir {
        if simple(&ir) {
            return ir;
        }
        let t = self.temp();
        self.c().pre.push(IrStmt::Set(t, ir));
        Ir::Local(t)
    }
    fn flush(&mut self, out: &mut Vec<IrStmt>) {
        out.append(&mut self.c().pre);
    }

    fn rt_id(&mut self, name: &str) -> u32 {
        if let Some(id) = self.rt_ids.get(name) {
            return *id;
        }
        let idx = self.prelude.funcs.iter().position(|f| f.name == name).unwrap_or_else(|| panic!("runtime function {name}"));
        let sig = self.signature(&self.prelude.funcs[idx], true).expect("prelude signatures are valid");
        let id = self.reserve(sig);
        self.rt_ids.insert(name.to_owned(), id);
        self.rt_work.push((idx, id));
        if matches!(name, "__task_spawn" | "__rx_subscribe" | "__rx_first_task") {
            self.needs_sched = true;
        }
        id
    }
    fn rt(&mut self, name: &str, args: Vec<Ir>) -> Ir {
        let id = self.rt_id(name);
        Ir::Call(id, args)
    }
    /// A call of a suspending function, hoisted to statement level.
    fn scall(&mut self, id: u32, args: Vec<Ir>) -> Ir {
        let args: Vec<Ir> = args.into_iter().map(|a| self.spill(a)).collect();
        let t = self.temp();
        let c = self.c();
        c.pre.push(IrStmt::SCall(Some(t), id, args));
        c.has_susp = true;
        Ir::Local(t)
    }
    fn rt_scall(&mut self, name: &str, args: Vec<Ir>) -> Ir {
        let id = self.rt_id(name);
        self.scall(id, args)
    }
    fn need_suspends(&self, pos: Pos, what: &str) -> R<()> {
        if self.cur().suspends {
            Ok(())
        } else {
            Err(err(pos, "E_AWAIT", format!("{what} is only valid inside a 'suspends' function, a task or an actor handler")))
        }
    }

    fn string(&mut self, s: &str, pos: Pos) -> R<Ir> {
        if self.data.len() + s.len() > MAX_STRING_DATA {
            return Err(err(pos, "E_SOURCE_LIMIT", "string literals exceed 32 KiB"));
        }
        let off = (DATA_BASE as usize + self.data.len()) as i64;
        self.data.extend_from_slice(s.as_bytes());
        Ok(Ir::Const((off << 32) | s.len() as i64))
    }

    fn tn(&self, t: &Ty) -> String {
        match t {
            Ty::Int => "int".into(),
            Ty::Bool => "bool".into(),
            Ty::Str => "string".into(),
            Ty::Void => "void".into(),
            Ty::Any => "Any".into(),
            Ty::Never => "never".into(),
            Ty::Raw => "word".into(),
            Ty::Infer => "_".into(),
            Ty::Option(a) => format!("Option<{}>", self.tn(a)),
            Ty::Result(a, b) => format!("Result<{},{}>", self.tn(a), self.tn(b)),
            Ty::Tuple(v) => format!("({})", v.iter().map(|x| self.tn(x)).collect::<Vec<_>>().join(", ")),
            Ty::List(a) => format!("List<{}>", self.tn(a)),
            Ty::Future(a) => format!("Future<{}>", self.tn(a)),
            Ty::Channel(a) => format!("Channel<{}>", self.tn(a)),
            Ty::AsyncGen(a) => format!("AsyncGenerator<{}>", self.tn(a)),
            Ty::Fn(p, r) => format!("fnc({}) -> {}", p.iter().map(|x| self.tn(x)).collect::<Vec<_>>().join(", "), self.tn(r)),
            Ty::Actor(i) => format!("ActorRef<{}>", self.objs[*i].name),
            Ty::Object(i) => self.objs[*i].name.clone(),
            Ty::Observable(a) => format!("Observable<{}>", self.tn(a)),
            Ty::Subscription => "Subscription".into(),
            Ty::Span => "Span".into(),
            Ty::Mail(a) => format!("ActorMail<{}>", self.tn(a)),
        }
    }

    fn resolve(&self, t: &TyEx, prelude: bool) -> R<Ty> {
        if prelude {
            return Ok(Ty::Raw);
        }
        let (name, args, pos) = match t {
            TyEx::Tuple(items, _) => {
                return Ok(Ty::Tuple(items.iter().map(|x| self.resolve(x, false)).collect::<R<Vec<_>>>()?));
            }
            TyEx::Name(n, a, p) => (n.as_str(), a, *p),
        };
        let arg = |i: usize| -> R<Box<Ty>> {
            let t = self.resolve(&args[i], false)?;
            if t == Ty::Void && !matches!(name, "Future" | "Task") {
                return Err(type_err(pos, "void is not a value type"));
            }
            Ok(Box::new(t))
        };
        Ok(match (name, args.len()) {
            ("int", 0) => Ty::Int,
            ("bool", 0) => Ty::Bool,
            ("string" | "String", 0) => Ty::Str,
            ("void", 0) => Ty::Void,
            ("Any", 0) => Ty::Any,
            ("Option", 1) => Ty::Option(arg(0)?),
            ("Result", 2) => Ty::Result(arg(0)?, arg(1)?),
            ("Future" | "Task", 1) => Ty::Future(arg(0)?),
            ("Channel", 1) => Ty::Channel(arg(0)?),
            ("AsyncGenerator", 1) => Ty::AsyncGen(arg(0)?),
            ("List" | "Array", 1) => Ty::List(arg(0)?),
            ("Observable", 1) => Ty::Observable(arg(0)?),
            ("Subscription", 0) => Ty::Subscription,
            ("Span", 0) => Ty::Span,
            ("ActorRef", 1) => match &args[0] {
                TyEx::Name(a, inner, _) if inner.is_empty() => match self.objs.iter().position(|o| o.name == *a && o.kind == ObjKind::Actor) {
                    Some(i) => Ty::Actor(i),
                    None => return Err(type_err(pos, format!("unknown actor '{a}'"))),
                },
                _ => return Err(type_err(pos, "ActorRef<A> names an actor")),
            },
            ("Generator", _) => return Err(unsupported(pos, "Generator<T> together with async/pattern features")),
            ("float" | "f64" | "double", 0) => return Err(unsupported(pos, "float values together with async/pattern features")),
            (n, 0) => match self.objs.iter().position(|o| o.name == n) {
                Some(i) if self.objs[i].kind == ObjKind::Actor => Ty::Actor(i),
                Some(i) => Ty::Object(i),
                None => return Err(type_err(pos, format!("unknown type '{n}'"))),
            },
            (n, _) => return Err(type_err(pos, format!("unknown generic type '{n}'"))),
        })
    }

    /// Convert a value of type `actual` for a context expecting `want`.
    fn coerce(&mut self, ir: Ir, actual: &Ty, want: &Ty, pos: Pos) -> R<Ir> {
        if *want == Ty::Any && !matches!(actual, Ty::Any | Ty::Raw | Ty::Never) {
            let tag = self.tag_of(actual, pos)?;
            return Ok(self.rt("__box", vec![Ir::Const(tag), ir]));
        }
        if !fits(actual, want) {
            return Err(type_err(pos, format!("expected {}, found {}", self.tn(want), self.tn(actual))));
        }
        Ok(ir)
    }

    fn tag_of(&self, t: &Ty, pos: Pos) -> R<i64> {
        match t {
            Ty::Int => Ok(TAG_INT),
            Ty::Bool => Ok(TAG_BOOL),
            Ty::Str => Ok(TAG_STR),
            other => Err(type_err(pos, format!("only int, bool and string values convert to Any, not {}", self.tn(other)))),
        }
    }

    // ---------------------------------------------------------- variables

    fn declare(&mut self, name: &str, ty: Ty, mutable: bool, pos: Pos) -> R<u32> {
        self.reserved_name(name, pos).or_else(|e| if self.cur().prelude { Ok(()) } else { Err(e) })?;
        if ty.has_hole() {
            return Err(type_err(pos, format!("cannot infer the type of '{name}'; add a type annotation")));
        }
        if ty == Ty::Void || ty == Ty::Never {
            return Err(type_err(pos, format!("'{name}' cannot hold a value of type {}", self.tn(&ty))));
        }
        let c = self.c();
        if c.scopes.last().unwrap().iter().any(|v| v.name == name) {
            return Err(err(pos, "E_DUPLICATE", format!("'{name}' is already declared in this scope")));
        }
        if c.nlocals >= MAX_LOCALS {
            return Err(err(pos, "E_SOURCE_LIMIT", "too many local variables"));
        }
        let slot = c.nlocals;
        c.nlocals += if matches!(ty, Ty::Mail(_)) { 2 } else { 1 };
        c.scopes.last_mut().unwrap().push(Var { name: name.to_owned(), slot, ty, mutable });
        Ok(slot)
    }

    fn find_at(&mut self, depth: usize, name: &str, pos: Pos) -> R<Option<Var>> {
        if let Some(v) = self.cx[depth].scopes.iter().rev().flat_map(|s| s.iter().rev()).find(|v| v.name == name) {
            return Ok(Some(v.clone()));
        }
        if !self.cx[depth].is_closure || depth == 0 {
            return Ok(None);
        }
        let Some(outer) = self.find_at(depth - 1, name, pos)? else {
            return Ok(None);
        };
        if outer.mutable {
            return Err(err(
                pos,
                "E_CAPTURE",
                format!("a task or lambda captures immutable bindings only; '{name}' is declared with 'let'"),
            ));
        }
        if matches!(outer.ty, Ty::Mail(_)) {
            return Err(err(pos, "E_CAPTURE", format!("copy '{name}.value' into a 'val' before using it in a task or lambda")));
        }
        let c = &mut self.cx[depth];
        let slot = c.nlocals;
        c.nlocals += 1;
        c.captures.push((outer.slot, slot));
        let v = Var { name: name.to_owned(), slot, ty: outer.ty, mutable: false };
        c.scopes[0].push(v.clone());
        Ok(Some(v))
    }

    fn find(&mut self, name: &str, pos: Pos) -> R<Option<Var>> {
        let depth = self.cx.len() - 1;
        self.find_at(depth, name, pos)
    }

    /// Read a variable, applying an active `is` refinement.
    fn read(&self, v: &Var) -> (Ir, Ty) {
        if let Some((_, t)) = self.cur().narrow.iter().rev().find(|(s, _)| *s == v.slot) {
            if v.ty == Ty::Any {
                return (load(Ir::Local(v.slot), 8), t.clone());
            }
        }
        (Ir::Local(v.slot), v.ty.clone())
    }

    /// Address and description of field `name` of the enclosing object.
    fn self_field(&mut self, name: &str, pos: Pos) -> R<Option<(Ir, FieldInfo)>> {
        let Some(this) = self.find("self", pos)? else {
            return Ok(None);
        };
        let (Ty::Actor(i) | Ty::Object(i)) = this.ty else {
            return Ok(None);
        };
        Ok(self.objs[i].fields.iter().find(|f| f.name == name).cloned().map(|f| (add(Ir::Local(this.slot), f.offset as i64), f)))
    }

    // ---------------------------------------------------------- functions

    fn lower_fn(&mut self, f: &Func, id: u32, owner: Option<usize>, prelude: bool) -> R<()> {
        let sig = self.sigs[id as usize].clone();
        if let Some(item) = &sig.async_gen {
            return self.lower_async_gen(f, id, &sig, item.clone());
        }
        let mut cx = FnCx::new(sig.ret.clone(), sig.suspends, prelude);
        self.cx.push(cx_take(&mut cx));
        if let Some(i) = owner {
            let ty = if self.objs[i].kind == ObjKind::Actor { Ty::Actor(i) } else { Ty::Object(i) };
            self.declare("self", ty, false, f.pos)?;
        }
        for (p, t) in f.params.iter().zip(&sig.params) {
            self.declare(&p.name, t.clone(), p.mutable, p.pos)?;
        }
        let params = self.cur().nlocals;
        let (body, diverges) = self.stmts(&f.body)?;
        let cx = self.cx.pop().unwrap();
        if sig.ret != Ty::Void && !diverges {
            return Err(err(f.pos, "E_RETURN", format!("function '{}' must return a value on every path", f.name)));
        }
        self.fns[id as usize] = Some(IrFn { name: f.name.clone(), params, locals: cx.nlocals, body, suspends: cx.has_susp });
        Ok(())
    }

    /// `suspends fnc f(..): AsyncGenerator<T>`: calling it starts a task that
    /// runs the body and feeds a one-slot channel; `yield` sends on it.
    fn lower_async_gen(&mut self, f: &Func, id: u32, sig: &FnSig, item: Ty) -> R<()> {
        self.cx.push(FnCx::new(sig.ret.clone(), false, false));
        for (p, t) in f.params.iter().zip(&sig.params) {
            self.declare(&p.name, t.clone(), false, p.pos)?;
        }
        let params = self.cur().nlocals;
        let ch = self.declare("yield channel", Ty::Channel(Box::new(item.clone())), false, f.pos)?;
        let mut body = vec![IrStmt::Set(ch, self.rt("__chan_new", vec![Ir::Const(1)]))];
        let (clo, _) = self.closure(&[], &f.body, Some(Ty::Void), true, &f.name, Some(item), f.pos)?;
        let spawn = self.rt("__task_spawn", vec![clo]);
        self.flush(&mut body);
        body.push(IrStmt::Expr(spawn));
        body.push(IrStmt::Return(Ir::Local(ch)));
        let cx = self.cx.pop().unwrap();
        self.fns[id as usize] = Some(IrFn { name: f.name.clone(), params, locals: cx.nlocals, body, suspends: false });
        Ok(())
    }

    /// Lower a lambda, task or generator body as its own function and build
    /// the closure value in the enclosing function. Returns the closure
    /// pointer and the body's result type.
    #[allow(clippy::too_many_arguments)]
    fn closure(
        &mut self,
        params: &[(String, Ty, Pos)],
        body: &[Stmt],
        ret: Option<Ty>,
        suspends: bool,
        name: &str,
        gen_item: Option<Ty>,
        pos: Pos,
    ) -> R<(Ir, Ty)> {
        let id = self.reserve(FnSig { params: Vec::new(), ret: Ty::Void, suspends, async_gen: None });
        let prelude = self.cur().prelude;
        let mut cx = FnCx::new(ret.clone().unwrap_or(Ty::Infer), suspends, prelude);
        cx.ret_infer = ret.is_none();
        cx.is_closure = true;
        cx.nlocals = 1; // slot 0: the closure itself (environment)
        self.cx.push(cx);
        for (n, t, p) in params {
            self.declare(n, t.clone(), false, *p)?;
        }
        let nparams = self.cur().nlocals;
        let mut prologue_cleanup = None;
        if let Some(item) = gen_item {
            let ch = self.find("yield channel", pos)?.expect("generator channel is declared");
            self.c().gen_chan = Some((ch.slot, item));
            let close = IrStmt::Expr(self.rt("__chan_close", vec![Ir::Local(ch.slot)]));
            self.c().cleanups.push(vec![close.clone()]);
            prologue_cleanup = Some(close);
        }
        let (mut stmts, diverges) = self.stmts(body)?;
        if let Some(close) = prologue_cleanup {
            self.c().cleanups.pop();
            stmts.push(close);
        }
        let cx = self.cx.pop().unwrap();
        let ret = if cx.ret_infer && cx.ret == Ty::Infer { Ty::Void } else { cx.ret.clone() };
        if ret != Ty::Void && !diverges {
            return Err(err(pos, "E_RETURN", "this block must return a value on every path"));
        }
        let mut full: Vec<IrStmt> =
            cx.captures.iter().enumerate().map(|(k, (_, inner))| IrStmt::Set(*inner, load(Ir::Local(0), 8 + 8 * k as i64))).collect();
        full.extend(stmts);
        self.fns[id as usize] =
            Some(IrFn { name: format!("{name}.closure"), params: nparams, locals: cx.nlocals, body: full, suspends: cx.has_susp });
        let slot = self.table.len() as i64;
        self.table.push(id);
        let t = self.temp();
        let c = self.c();
        c.pre.push(IrStmt::Set(t, Ir::Helper(Helper::Alloc, vec![Ir::Const(8 + 8 * cx.captures.len() as i64)])));
        c.pre.push(IrStmt::Store(Ir::Local(t), Ir::Const(slot)));
        for (k, (outer, _)) in cx.captures.iter().enumerate() {
            c.pre.push(IrStmt::Store(add(Ir::Local(t), 8 + 8 * k as i64), Ir::Local(*outer)));
        }
        Ok((Ir::Local(t), ret))
    }

    // ------------------------------------------------------------ objects

    fn lower_ctor(&mut self, i: usize) -> R<()> {
        let o = &self.prog.objects[i];
        let actor = o.kind == ObjKind::Actor;
        let ty = if actor { Ty::Actor(i) } else { Ty::Object(i) };
        self.cx.push(FnCx::new(ty.clone(), false, false));
        let this = self.declare("self", ty, false, o.pos)?;
        let base = if actor { 24 } else { 0 };
        let size = base + 8 * o.fields.len() as u32;
        self.objs[i].size = size;
        let mut body = vec![IrStmt::Set(this, Ir::Helper(Helper::Alloc, vec![Ir::Const(size.max(8) as i64)]))];
        if actor {
            let mailbox = self.rt("__chan_new", vec![Ir::Const(-1)]);
            body.push(IrStmt::Store(Ir::Local(this), mailbox));
        }
        for (k, f) in o.fields.iter().enumerate() {
            self.reserved_name(&f.name, f.pos)?;
            if self.objs[i].fields.iter().any(|x| x.name == f.name) {
                return Err(err(f.pos, "E_DUPLICATE", format!("field '{}' is already declared", f.name)));
            }
            let want = match &f.ty {
                Some(t) => Some(self.resolve(t, false)?),
                None => None,
            };
            let (ir, ty) = self.value(&f.init, want.as_ref())?;
            let ty = match &want {
                Some(w) => {
                    let _ = self.coerce(Ir::Const(0), &ty, w, f.init.pos)?;
                    w.clone()
                }
                None => ty,
            };
            let ir = match &want {
                Some(w) => self.coerce(ir, &ty, w, f.init.pos)?,
                None => ir,
            };
            if ty.has_hole() {
                return Err(type_err(f.pos, format!("cannot infer the type of field '{}'; add a type annotation", f.name)));
            }
            self.flush(&mut body);
            let offset = base + 8 * k as u32;
            body.push(IrStmt::Store(add(Ir::Local(this), offset as i64), ir));
            self.objs[i].fields.push(FieldInfo { name: f.name.clone(), ty, mutable: f.mutable, offset });
        }
        if actor {
            // start the actor's mailbox loop as a task
            let run = self.actor_loop(i);
            let slot = self.table.len() as i64;
            self.table.push(run);
            let clo = self.temp();
            body.push(IrStmt::Set(clo, Ir::Helper(Helper::Alloc, vec![Ir::Const(16)])));
            body.push(IrStmt::Store(Ir::Local(clo), Ir::Const(slot)));
            body.push(IrStmt::Store(add(Ir::Local(clo), 8), Ir::Local(this)));
            let spawn = self.rt("__task_spawn", vec![Ir::Local(clo)]);
            body.push(IrStmt::Expr(spawn));
        }
        body.push(IrStmt::Return(Ir::Local(this)));
        let cx = self.cx.pop().unwrap();
        let ctor = self.objs[i].ctor;
        self.fns[ctor as usize] = Some(IrFn { name: format!("{}.new", o.name), params: 0, locals: cx.nlocals, body, suspends: false });

        // dispose: stop every actor this object owns through a field
        let mut stop = Vec::new();
        let fields = self.objs[i].fields.clone();
        for f in &fields {
            if matches!(f.ty, Ty::Actor(_)) {
                let call = self.rt("__actor_stop", vec![load(Ir::Local(0), f.offset as i64)]);
                stop.push(IrStmt::Expr(call));
            }
        }
        let dispose = self.objs[i].dispose;
        self.fns[dispose as usize] =
            Some(IrFn { name: format!("{}.dispose", o.name), params: 1, locals: 1, body: stop, suspends: false });
        Ok(())
    }

    /// `loop { mail = next message or stop; receive(self, mail, sequence) }`
    fn actor_loop(&mut self, i: usize) -> u32 {
        let id = self.reserve(FnSig { params: vec![Ty::Raw], ret: Ty::Raw, suspends: true, async_gen: None });
        let recv = self.rt_id("__chan_recv_opt");
        // locals: 0 env, 1 self, 2 mailbox, 3 option, 4 value, 5 sequence
        let mut body = vec![IrStmt::Set(1, load(Ir::Local(0), 8)), IrStmt::Set(2, load(Ir::Local(1), 0))];
        let mut turn = vec![
            IrStmt::SCall(Some(3), recv, vec![Ir::Local(2)]),
            IrStmt::If(eq(Ir::Local(3), Ir::Const(0)), vec![IrStmt::Return(Ir::Const(0))], Vec::new()),
            IrStmt::Set(4, load(Ir::Local(3), 0)),
            IrStmt::Set(5, load(Ir::Local(1), 16)),
            IrStmt::Store(add(Ir::Local(1), 16), add(Ir::Local(5), 1)),
        ];
        if let Some(receive) = self.objs[i].receive {
            turn.push(IrStmt::SCall(None, receive, vec![Ir::Local(1), Ir::Local(4), Ir::Local(5)]));
        }
        body.push(IrStmt::Loop { body: turn, update: Vec::new() });
        self.fns[id as usize] =
            Some(IrFn { name: format!("{}.mailbox", self.objs[i].name), params: 1, locals: 6, body, suspends: true });
        id
    }

    fn lower_receive(&mut self, i: usize, r: &Receive) -> R<()> {
        let id = self.objs[i].receive.unwrap();
        self.cx.push(FnCx::new(Ty::Void, true, false));
        self.declare("self", Ty::Actor(i), false, r.pos)?;
        let mail = self.objs[i].mail.clone();
        // slots 1 and 2: the mail value and its sequence number
        self.declare(&r.mail, Ty::Mail(Box::new(mail)), false, r.pos)?;
        let (body, _) = self.stmts(&r.body)?;
        let cx = self.cx.pop().unwrap();
        self.fns[id as usize] = Some(IrFn {
            name: format!("{}.receive", self.objs[i].name),
            params: 3,
            locals: cx.nlocals,
            body,
            suspends: cx.has_susp,
        });
        Ok(())
    }

    // --------------------------------------------------------- statements

    fn stmts(&mut self, body: &[Stmt]) -> R<(Vec<IrStmt>, bool)> {
        self.c().scopes.push(Vec::new());
        let narrow = self.cur().narrow.len();
        // Statements nested in an expression (match arms) must not flush the
        // statements still pending for that enclosing expression.
        let pending = std::mem::take(&mut self.c().pre);
        let mut out = Vec::new();
        let mut diverges = false;
        let mut failed = None;
        for s in body {
            match self.stmt(s, &mut out) {
                Ok(d) => diverges |= d,
                Err(e) => {
                    failed = Some(e);
                    break;
                }
            }
        }
        let c = self.c();
        c.pre = pending;
        c.scopes.pop();
        c.narrow.truncate(narrow);
        match failed {
            Some(e) => Err(e),
            None => Ok((out, diverges)),
        }
    }

    /// Lower one statement into `out`; true when control never continues
    /// past it.
    fn stmt(&mut self, s: &Stmt, out: &mut Vec<IrStmt>) -> R<bool> {
        match s {
            Stmt::Bind { kind, target, ty, init, pos } => {
                let want = match ty {
                    Some(t) => Some(self.resolve(t, self.cur().prelude)?),
                    None => None,
                };
                let (ir, actual) = self.value(init, want.as_ref())?;
                let (ir, bound) = match &want {
                    Some(w) => (self.coerce(ir, &actual, w, init.pos)?, if *w == Ty::Raw { Ty::Raw } else { merge(w, &actual) }),
                    None => (ir, actual),
                };
                let bound = if want.as_ref() == Some(&Ty::Any) { Ty::Any } else { bound };
                self.flush(out);
                let mutable = *kind == BindKind::Let;
                match target {
                    BindTarget::Name(name) => {
                        let slot = self.declare(name, bound, mutable, *pos)?;
                        out.push(IrStmt::Set(slot, ir));
                    }
                    BindTarget::Tuple(names) => {
                        let Ty::Tuple(items) = bound else {
                            return Err(type_err(*pos, format!("only a tuple can be destructured, not {}", self.tn(&bound))));
                        };
                        if items.len() != names.len() {
                            return Err(type_err(*pos, format!("tuple has {} elements, {} names given", items.len(), names.len())));
                        }
                        let t = self.temp();
                        out.push(IrStmt::Set(t, ir));
                        for (k, (n, ty)) in names.iter().zip(items).enumerate() {
                            let slot = self.declare(n, ty, mutable, *pos)?;
                            out.push(IrStmt::Set(slot, load(Ir::Local(t), 8 * k as i64)));
                        }
                    }
                }
                Ok(false)
            }
            Stmt::Assign { name, value, pos } => {
                if let Some(v) = self.find(name, *pos)? {
                    if !v.mutable {
                        return Err(err(*pos, "E_IMMUTABLE", format!("cannot reassign immutable binding '{name}'")));
                    }
                    let ir = self.typed(value, &v.ty)?;
                    self.flush(out);
                    out.push(IrStmt::Set(v.slot, ir));
                    return Ok(false);
                }
                if let Some((addr, f)) = self.self_field(name, *pos)? {
                    return self.store_field(addr, &f, value, *pos, out);
                }
                Err(err(*pos, "E_UNDEFINED", format!("unknown variable '{name}'")))
            }
            Stmt::FieldAssign { target, field, value, pos } => {
                if !matches!(target.kind, Ex::SelfRef) {
                    return Err(err(*pos, "E_ASSIGN_TARGET", "only a field of 'self' can be assigned"));
                }
                match self.self_field(field, *pos)? {
                    Some((addr, f)) => self.store_field(addr, &f, value, *pos, out),
                    None => Err(err(*pos, "E_UNDEFINED", format!("unknown field '{field}'"))),
                }
            }
            Stmt::Expr(e) => {
                let (ir, ty) = self.expr(e, None)?;
                self.flush(out);
                if !simple(&ir) {
                    out.push(IrStmt::Expr(ir));
                }
                Ok(ty == Ty::Never)
            }
            Stmt::Return(value, pos) => {
                let want = self.cur().ret.clone();
                let ir = match value {
                    None => {
                        if !matches!(want, Ty::Void | Ty::Infer | Ty::Raw) {
                            return Err(err(*pos, "E_RETURN", format!("this function returns {}", self.tn(&want))));
                        }
                        if want == Ty::Infer {
                            self.c().ret = Ty::Void;
                        }
                        Ir::Const(0)
                    }
                    Some(e) => {
                        let hint = if want == Ty::Infer { None } else { Some(&want) };
                        let (ir, ty) = self.expr(e, hint)?;
                        if ty == Ty::Void && !matches!(want, Ty::Void | Ty::Infer) {
                            return Err(type_err(e.pos, "this expression has no value"));
                        }
                        if ty == Ty::Void {
                            // `return f();` of a void call: run it, return nothing
                            if want == Ty::Infer {
                                self.c().ret = Ty::Void;
                            }
                            if !simple(&ir) {
                                self.c().pre.push(IrStmt::Expr(ir));
                            }
                            Ir::Const(0)
                        } else if self.cur().ret_infer {
                            let known = self.cur().ret.clone();
                            if !fits(&ty, &known) {
                                return Err(type_err(e.pos, format!("expected {}, found {}", self.tn(&known), self.tn(&ty))));
                            }
                            self.c().ret = merge(&known, &ty);
                            ir
                        } else {
                            self.coerce(ir, &ty, &want, e.pos)?
                        }
                    }
                };
                self.flush(out);
                let all: Vec<IrStmt> = self.cur().cleanups.iter().rev().flatten().cloned().collect();
                if all.is_empty() {
                    out.push(IrStmt::Return(ir));
                } else {
                    let t = self.temp();
                    out.push(IrStmt::Set(t, ir));
                    out.extend(all);
                    out.push(IrStmt::Return(Ir::Local(t)));
                }
                Ok(true)
            }
            Stmt::If { arms, otherwise, .. } => {
                let mut diverges = otherwise.is_some();
                let mut lowered = Vec::new();
                for (cond, body) in arms {
                    let mut pre = Vec::new();
                    let c = self.condition(cond)?;
                    self.flush(&mut pre);
                    let refined = self.narrowings(cond)?;
                    let mark = self.cur().narrow.len();
                    self.c().narrow.extend(refined);
                    let (body, d) = self.stmts(body)?;
                    self.c().narrow.truncate(mark);
                    diverges &= d;
                    lowered.push((pre, c, body));
                }
                let mut tail = match otherwise {
                    Some(body) => {
                        let (body, d) = self.stmts(body)?;
                        diverges &= d;
                        body
                    }
                    None => Vec::new(),
                };
                for (mut pre, c, body) in lowered.into_iter().rev() {
                    pre.push(IrStmt::If(c, body, tail));
                    tail = pre;
                }
                out.extend(tail);
                Ok(diverges)
            }
            Stmt::While { cond, body, .. } => {
                let mut inner = Vec::new();
                let c = self.condition(cond)?;
                self.flush(&mut inner);
                inner.push(IrStmt::If(not(c), vec![IrStmt::Break], Vec::new()));
                let (b, _, _) = self.loop_body(body)?;
                inner.extend(b);
                out.push(IrStmt::Loop { body: inner, update: Vec::new() });
                Ok(false)
            }
            Stmt::Loop { body, .. } => {
                let (b, _, broke) = self.loop_body(body)?;
                out.push(IrStmt::Loop { body: b, update: Vec::new() });
                Ok(!broke)
            }
            Stmt::For { init, cond, update, body, .. } => {
                self.c().scopes.push(Vec::new());
                if let Some(i) = init {
                    self.stmt(i, out)?;
                }
                let mut inner = Vec::new();
                if let Some(cond) = cond {
                    let c = self.condition(cond)?;
                    self.flush(&mut inner);
                    inner.push(IrStmt::If(not(c), vec![IrStmt::Break], Vec::new()));
                }
                let (b, _, broke) = self.loop_body(body)?;
                inner.extend(b);
                let mut step = Vec::new();
                if let Some(u) = update {
                    self.stmt(u, &mut step)?;
                }
                self.c().scopes.pop();
                out.push(IrStmt::Loop { body: inner, update: step });
                Ok(cond.is_none() && !broke)
            }
            Stmt::ForIn { name, source, body, is_await, pos } => self.for_in(name, source, body, *is_await, *pos, out),
            Stmt::Break(pos) | Stmt::Continue(pos) => {
                let Some(&(height, _)) = self.cur().loops.last() else {
                    return Err(err(*pos, "E_LOOP", "'break' and 'continue' are only valid inside a loop"));
                };
                let pending: Vec<IrStmt> = self.cur().cleanups[height..].iter().rev().flatten().cloned().collect();
                out.extend(pending);
                if matches!(s, Stmt::Break(_)) {
                    self.c().loops.last_mut().unwrap().1 = true;
                    out.push(IrStmt::Break);
                } else {
                    out.push(IrStmt::Continue);
                }
                Ok(true)
            }
            Stmt::Block(body) => {
                let (b, d) = self.stmts(body)?;
                out.extend(b);
                Ok(d)
            }
            Stmt::Yield(value, pos) => {
                let Some((ch, item)) = self.cur().gen_chan.clone() else {
                    return Err(err(*pos, "E_YIELD", "'yield' is only valid in a 'suspends' function returning AsyncGenerator<T>"));
                };
                let ir = self.typed(value, &item)?;
                self.rt_scall("__chan_send", vec![Ir::Local(ch), ir]);
                self.flush(out);
                Ok(false)
            }
            Stmt::YieldFrom(_, pos) => Err(unsupported(*pos, "'yield*' inside an async generator")),
            Stmt::WriteCh { ch, value, pos } => {
                self.need_suspends(*pos, "'writech'")?;
                let (c, ty) = self.value(ch, None)?;
                let Ty::Channel(item) = ty else {
                    return Err(type_err(ch.pos, format!("'writech' needs a Channel<T>, found {}", self.tn(&ty))));
                };
                let c = self.spill(c);
                let v = self.typed(value, &item)?;
                self.rt_scall("__chan_send", vec![c, v]);
                self.flush(out);
                Ok(false)
            }
            Stmt::DoTask(body, pos) => {
                let (clo, _) = self.closure(&[], body, None, true, "task", None, *pos)?;
                let spawn = self.rt("__task_spawn", vec![clo]);
                self.flush(out);
                out.push(IrStmt::Expr(spawn));
                Ok(false)
            }
            Stmt::Using { resource, name, body, pos } => {
                let (ir, ty) = self.value(resource, None)?;
                self.flush(out);
                self.c().scopes.push(Vec::new());
                let slot = match name {
                    Some(n) => self.declare(n, ty.clone(), false, *pos)?,
                    None => self.temp(),
                };
                out.push(IrStmt::Set(slot, ir));
                let release = match &ty {
                    Ty::Span => self.rt("__span_close", vec![Ir::Local(slot)]),
                    Ty::Subscription => self.rt("__sub_dispose", vec![Ir::Local(slot)]),
                    Ty::Channel(_) => self.rt("__chan_close", vec![Ir::Local(slot)]),
                    Ty::Object(i) => Ir::Call(self.objs[*i].dispose, vec![Ir::Local(slot)]),
                    other => {
                        return Err(type_err(resource.pos, format!("'using' needs a closeable resource, found {}", self.tn(other))));
                    }
                };
                let d = self.scoped_cleanup(vec![IrStmt::Expr(release)], body, out)?;
                self.c().scopes.pop();
                Ok(d)
            }
            Stmt::With { scope, body, pos } => {
                let Ex::Call { name, args, .. } = &scope.kind else {
                    return Err(unsupported(*pos, "'with' scopes other than timeout(ms)"));
                };
                if name != "timeout" || args.len() != 1 || args[0].name.is_some() {
                    return Err(unsupported(*pos, "'with' scopes other than timeout(ms)"));
                }
                self.need_suspends(*pos, "'with timeout'")?;
                let ms = self.typed(&args[0].value, &Ty::Int)?;
                let old = self.temp();
                let push = self.rt("__deadline_push", vec![ms]);
                self.flush(out);
                out.push(IrStmt::Set(old, push));
                let pop = self.rt("__deadline_pop", vec![Ir::Local(old)]);
                self.scoped_cleanup(vec![IrStmt::Expr(pop)], body, out)
            }
        }
    }

    /// Run `body` with `cleanup` registered for early exits, then run it on
    /// the normal path.
    fn scoped_cleanup(&mut self, cleanup: Vec<IrStmt>, body: &[Stmt], out: &mut Vec<IrStmt>) -> R<bool> {
        self.c().cleanups.push(cleanup.clone());
        let (b, d) = self.stmts(body)?;
        self.c().cleanups.pop();
        out.extend(b);
        if !d {
            out.extend(cleanup);
        }
        Ok(d)
    }

    fn store_field(&mut self, addr: Ir, f: &FieldInfo, value: &Expr, pos: Pos, out: &mut Vec<IrStmt>) -> R<bool> {
        if !f.mutable {
            return Err(err(pos, "E_IMMUTABLE", format!("cannot reassign immutable field '{}'", f.name)));
        }
        let ir = self.typed(value, &f.ty)?;
        self.flush(out);
        out.push(IrStmt::Store(addr, ir));
        Ok(false)
    }

    /// (statements, diverges, has break)
    fn loop_body(&mut self, body: &[Stmt]) -> R<(Vec<IrStmt>, bool, bool)> {
        let height = self.cur().cleanups.len();
        self.c().loops.push((height, false));
        let r = self.stmts(body);
        let (_, broke) = self.c().loops.pop().unwrap();
        let (b, d) = r?;
        Ok((b, d, broke))
    }

    fn for_in(&mut self, name: &str, source: &Expr, body: &[Stmt], is_await: bool, pos: Pos, out: &mut Vec<IrStmt>) -> R<bool> {
        let (src, ty) = self.value(source, None)?;
        self.flush(out);
        self.c().scopes.push(Vec::new());
        let s = self.temp();
        out.push(IrStmt::Set(s, src));
        let mut inner = Vec::new();
        let mut step = Vec::new();
        match (&ty, is_await) {
            (Ty::AsyncGen(item) | Ty::Channel(item), true) => {
                self.need_suspends(pos, "'for await'")?;
                let next = self.rt_scall("__chan_recv_opt", vec![Ir::Local(s)]);
                self.flush(&mut inner);
                inner.push(IrStmt::If(eq(next.clone(), Ir::Const(0)), vec![IrStmt::Break], Vec::new()));
                let slot = self.declare(name, (**item).clone(), false, pos)?;
                inner.push(IrStmt::Set(slot, load(next, 0)));
            }
            (Ty::List(item), false) => {
                let i = self.temp();
                out.push(IrStmt::Set(i, Ir::Const(0)));
                let done = Ir::Bin(Op::GeS, Box::new(Ir::Local(i)), Box::new(load(Ir::Local(s), 0)));
                inner.push(IrStmt::If(done, vec![IrStmt::Break], Vec::new()));
                let slot = self.declare(name, (**item).clone(), false, pos)?;
                let at = Ir::Bin(
                    Op::AddRaw,
                    Box::new(Ir::Local(s)),
                    Box::new(Ir::Bin(Op::MulRaw, Box::new(add(Ir::Local(i), 1)), Box::new(Ir::Const(8)))),
                );
                inner.push(IrStmt::Set(slot, Ir::Load(Box::new(at))));
                step.push(IrStmt::Set(i, add(Ir::Local(i), 1)));
            }
            (Ty::AsyncGen(_) | Ty::Channel(_), false) => {
                return Err(type_err(source.pos, "iterate an async source with 'for await'"));
            }
            (other, _) => {
                return Err(type_err(source.pos, format!("cannot iterate over {}", self.tn(other))));
            }
        }
        let (b, _, _) = self.loop_body(body)?;
        self.c().scopes.pop();
        inner.extend(b);
        out.push(IrStmt::Loop { body: inner, update: step });
        Ok(false)
    }

    // -------------------------------------------------------- expressions

    /// A value of exactly type `want` (after coercion).
    fn typed(&mut self, e: &Expr, want: &Ty) -> R<Ir> {
        let (ir, ty) = self.value(e, Some(want))?;
        self.coerce(ir, &ty, want, e.pos)
    }

    fn value(&mut self, e: &Expr, want: Option<&Ty>) -> R<(Ir, Ty)> {
        let (ir, ty) = self.expr(e, want)?;
        if ty == Ty::Void {
            return Err(type_err(e.pos, "this expression has no value"));
        }
        Ok((ir, ty))
    }

    fn condition(&mut self, e: &Expr) -> R<Ir> {
        let (ir, ty) = self.value(e, Some(&Ty::Bool))?;
        if !matches!(ty, Ty::Bool | Ty::Raw | Ty::Never) {
            return Err(type_err(e.pos, format!("a condition is a bool, found {}", self.tn(&ty))));
        }
        Ok(ir)
    }

    /// Refinements that hold when `e` is true: `x is T` and conjunctions.
    fn narrowings(&mut self, e: &Expr) -> R<Vec<(u32, Ty)>> {
        Ok(match &e.kind {
            Ex::Is(inner, ty) => match &inner.kind {
                Ex::Var(n) => match self.find(n, inner.pos)? {
                    Some(v) if v.ty == Ty::Any => vec![(v.slot, self.resolve(ty, false)?)],
                    _ => Vec::new(),
                },
                _ => Vec::new(),
            },
            Ex::Binary(BinOp::And | BinOp::CondAnd, a, b) => {
                let mut v = self.narrowings(a)?;
                v.extend(self.narrowings(b)?);
                v
            }
            _ => Vec::new(),
        })
    }

    /// Lower operands left to right. When a later operand hoists statements,
    /// earlier operands are spilled first so evaluation order is preserved.
    fn operands(&mut self, items: &[(&Expr, Option<Ty>)]) -> R<Vec<(Ir, Ty)>> {
        let mut out: Vec<(Ir, Ty)> = Vec::new();
        for (e, want) in items {
            let mark = self.cur().pre.len();
            let (ir, ty) = self.value(e, want.as_ref())?;
            if self.cur().pre.len() > mark {
                let mut spills = Vec::new();
                for (prev, _) in out.iter_mut() {
                    if !matches!(prev, Ir::Const(_)) {
                        let t = self.temp();
                        spills.push(IrStmt::Set(t, std::mem::replace(prev, Ir::Local(t))));
                    }
                }
                let c = self.c();
                let tail = c.pre.split_off(mark);
                c.pre.extend(spills);
                c.pre.extend(tail);
            }
            out.push((ir, ty));
        }
        Ok(out)
    }

    /// Lower `e` with its hoisted statements captured separately.
    fn isolated(&mut self, e: &Expr, want: Option<&Ty>) -> R<(Vec<IrStmt>, Ir, Ty)> {
        let saved = std::mem::take(&mut self.c().pre);
        let r = self.expr(e, want);
        let pre = std::mem::replace(&mut self.c().pre, saved);
        let (ir, ty) = r?;
        Ok((pre, ir, ty))
    }

    fn expr(&mut self, e: &Expr, want: Option<&Ty>) -> R<(Ir, Ty)> {
        let pos = e.pos;
        match &e.kind {
            Ex::Int(Some(v)) => Ok((Ir::Const(*v as i64), Ty::Int)),
            Ex::Int(None) => Err(err(pos, "E_INTEGER_RANGE", "integer literal does not fit in a signed 64-bit int")),
            Ex::Bool(b) => Ok((Ir::Const(*b as i64), Ty::Bool)),
            Ex::Str(s) => Ok((self.string(s, pos)?, Ty::Str)),
            Ex::Var(name) => self.var(name, want, pos),
            Ex::SelfRef => match self.find("self", pos)? {
                Some(v) => Ok((Ir::Local(v.slot), v.ty)),
                None => Err(err(pos, "E_SELF", "'self' is only valid inside an actor or supervisor")),
            },
            Ex::Unary(op, inner) => self.unary(*op, inner, pos),
            Ex::Binary(op, l, r) => self.binary(*op, l, r, pos),
            Ex::Ternary(c, a, b) => {
                let cond = self.condition(c)?;
                let (pa, a_ir, a_ty) = self.isolated(a, want)?;
                let (pb, b_ir, b_ty) = self.isolated(b, want.or(Some(&a_ty)))?;
                if !fits(&b_ty, &a_ty) || a_ty == Ty::Void {
                    return Err(type_err(pos, format!("ternary arms differ: {} and {}", self.tn(&a_ty), self.tn(&b_ty))));
                }
                let ty = merge(&a_ty, &b_ty);
                if pa.is_empty() && pb.is_empty() {
                    return Ok((Ir::If(Box::new(cond), Box::new(a_ir), Box::new(b_ir)), ty));
                }
                let t = self.temp();
                let mut ta = pa;
                ta.push(IrStmt::Set(t, a_ir));
                let mut tb = pb;
                tb.push(IrStmt::Set(t, b_ir));
                self.c().pre.push(IrStmt::If(cond, ta, tb));
                Ok((Ir::Local(t), ty))
            }
            Ex::Call { name, targs, args } => {
                let (ir, ty, _) = self.call(name, targs, args, want, pos, false)?;
                Ok((ir, ty))
            }
            Ex::Method { base, name, args } => {
                let (ir, ty, _) = self.method(base, name, args, pos, false)?;
                Ok((ir, ty))
            }
            Ex::Member(base, name) => self.member(base, name, pos),
            Ex::TypeStatic { ty, name, args } => {
                let t = self.resolve(ty, false)?;
                let Ty::Channel(_) = t else {
                    return Err(type_err(pos, format!("{} has no static member '{name}'", self.tn(&t))));
                };
                match (name.as_str(), args.len()) {
                    ("bounded", 1) if args[0].name.is_none() => {
                        let n = self.typed(&args[0].value, &Ty::Int)?;
                        Ok((self.rt("__chan_new", vec![n]), t))
                    }
                    ("unbounded", 0) => Ok((self.rt("__chan_new", vec![Ir::Const(-1)]), t)),
                    _ => Err(type_err(pos, "a channel is created with Channel<T>.bounded(n) or Channel<T>.unbounded()")),
                }
            }
            Ex::Tuple(items) => {
                let wants: Vec<Option<Ty>> = match want {
                    Some(Ty::Tuple(w)) if w.len() == items.len() => w.iter().cloned().map(Some).collect(),
                    _ => vec![None; items.len()],
                };
                let pairs: Vec<(&Expr, Option<Ty>)> = items.iter().zip(wants).collect();
                let vals = self.operands(&pairs)?;
                let t = self.temp();
                self.c().pre.push(IrStmt::Set(t, Ir::Helper(Helper::Alloc, vec![Ir::Const(8 * items.len() as i64)])));
                let mut tys = Vec::new();
                for (k, (ir, ty)) in vals.into_iter().enumerate() {
                    self.c().pre.push(IrStmt::Store(add(Ir::Local(t), 8 * k as i64), ir));
                    tys.push(ty);
                }
                Ok((Ir::Local(t), Ty::Tuple(tys)))
            }
            Ex::List(items) => {
                let mut item = match want {
                    Some(Ty::List(t)) => (**t).clone(),
                    _ => Ty::Infer,
                };
                let hint = if item == Ty::Infer { None } else { Some(item.clone()) };
                let pairs: Vec<(&Expr, Option<Ty>)> = items.iter().map(|x| (x, hint.clone())).collect();
                let vals = self.operands(&pairs)?;
                let t = self.temp();
                self.c().pre.push(IrStmt::Set(t, Ir::Helper(Helper::Alloc, vec![Ir::Const(8 + 8 * items.len() as i64)])));
                self.c().pre.push(IrStmt::Store(Ir::Local(t), Ir::Const(items.len() as i64)));
                for (k, ((ir, ty), src)) in vals.into_iter().zip(items).enumerate() {
                    if !fits(&ty, &item) {
                        return Err(type_err(src.pos, format!("list elements differ: {} and {}", self.tn(&item), self.tn(&ty))));
                    }
                    item = merge(&item, &ty);
                    self.c().pre.push(IrStmt::Store(add(Ir::Local(t), 8 + 8 * k as i64), ir));
                }
                Ok((Ir::Local(t), Ty::List(Box::new(item))))
            }
            Ex::Lambda { params, ret, body } => {
                let (want_params, want_ret) = match want {
                    Some(Ty::Fn(p, r)) => (Some(p.clone()), Some((**r).clone())),
                    _ => (None, None),
                };
                if let Some(w) = &want_params {
                    if w.len() != params.len() {
                        return Err(type_err(pos, format!("this lambda takes {} parameter(s), {} expected", params.len(), w.len())));
                    }
                }
                let mut typed = Vec::new();
                for (k, p) in params.iter().enumerate() {
                    let t = match (&p.ty, &want_params) {
                        (Some(t), _) => self.resolve(t, self.cur().prelude)?,
                        (None, Some(w)) if !w[k].has_hole() => w[k].clone(),
                        _ => return Err(type_err(p.pos, format!("cannot infer the type of parameter '{}'; write '|{}: T|'", p.name, p.name))),
                    };
                    typed.push((p.name.clone(), t, p.pos));
                }
                let declared = match ret {
                    Some(t) => Some(self.resolve(t, self.cur().prelude)?),
                    None => want_ret.filter(|r| !r.has_hole()),
                };
                let (clo, r) = self.closure(&typed, body, declared, false, "lambda", None, pos)?;
                Ok((clo, Ty::Fn(typed.into_iter().map(|p| p.1).collect(), Box::new(r))))
            }
            Ex::Await(inner) => self.await_expr(inner, pos),
            Ex::Task(body) => {
                let (clo, ret) = self.closure(&[], body, None, true, "task", None, pos)?;
                Ok((self.rt("__task_spawn", vec![clo]), Ty::Future(Box::new(ret))))
            }
            Ex::Spawn(name, args) => {
                let Some(i) = self.objs.iter().position(|o| o.name == *name && o.kind == ObjKind::Actor) else {
                    return Err(err(pos, "E_UNDEFINED", format!("unknown actor '{name}'")));
                };
                if !args.is_empty() {
                    return Err(err(pos, "E_ACTOR", format!("actor '{name}' does not accept startup arguments; initialize state with fields")));
                }
                self.needs_sched = true;
                Ok((Ir::Call(self.objs[i].ctor, Vec::new()), Ty::Actor(i)))
            }
            Ex::ReadCh(mode, ch) => {
                let (c, ty) = self.value(ch, None)?;
                let Ty::Channel(item) = ty else {
                    return Err(type_err(ch.pos, format!("'readch' needs a Channel<T>, found {}", self.tn(&ty))));
                };
                match mode {
                    ReadMode::Wait => {
                        self.need_suspends(pos, "'readch'")?;
                        Ok((self.rt_scall("__chan_recv", vec![c]), *item))
                    }
                    ReadMode::Try => Ok((self.rt("__chan_try", vec![c]), Ty::Option(item))),
                    ReadMode::Nb => Ok((self.rt("__chan_nb", vec![c]), Ty::Future(Box::new(Ty::Option(item))))),
                }
            }
            Ex::Match(subject, arms) => self.match_expr(subject, arms, want, pos),
            Ex::Is(inner, ty) => {
                let target = self.resolve(ty, false)?;
                if let Ex::Var(n) = &inner.kind {
                    if let Some(v) = self.find(n, inner.pos)? {
                        if v.ty == Ty::Any {
                            let tag = self.tag_of(&target, ty.pos())?;
                            return Ok((eq(load(Ir::Local(v.slot), 0), Ir::Const(tag)), Ty::Bool));
                        }
                    }
                }
                let (ir, actual) = self.value(inner, None)?;
                if actual == Ty::Any {
                    let tag = self.tag_of(&target, ty.pos())?;
                    return Ok((eq(load(ir, 0), Ir::Const(tag)), Ty::Bool));
                }
                Ok((Ir::Const((actual == target) as i64), Ty::Bool))
            }
        }
    }

    fn var(&mut self, name: &str, want: Option<&Ty>, pos: Pos) -> R<(Ir, Ty)> {
        if let Some(v) = self.find(name, pos)? {
            return Ok(self.read(&v));
        }
        if let Some((addr, f)) = self.self_field(name, pos)? {
            return Ok((Ir::Load(Box::new(addr)), f.ty));
        }
        if name == "None" {
            let inner = match want {
                Some(Ty::Option(t)) => (**t).clone(),
                _ => Ty::Infer,
            };
            return Ok((Ir::Const(0), Ty::Option(Box::new(inner))));
        }
        Err(err(pos, "E_UNDEFINED", format!("unknown variable '{name}'")))
    }

    fn is_int(&self, t: &Ty) -> bool {
        matches!(t, Ty::Int | Ty::Raw | Ty::Never)
    }
    fn is_bool(&self, t: &Ty) -> bool {
        matches!(t, Ty::Bool | Ty::Raw | Ty::Never)
    }

    fn unary(&mut self, op: UnOp, inner: &Expr, pos: Pos) -> R<(Ir, Ty)> {
        if op == UnOp::Neg {
            if let Ex::Int(None) = inner.kind {
                return Ok((Ir::Const(i64::MIN), Ty::Int));
            }
            if let Ex::Int(Some(v)) = inner.kind {
                return Ok((Ir::Const(-(v as i64)), Ty::Int));
            }
        }
        let (ir, ty) = self.value(inner, None)?;
        match op {
            UnOp::Not => {
                if !self.is_bool(&ty) {
                    return Err(type_err(pos, format!("'!' needs a bool, found {}", self.tn(&ty))));
                }
                Ok((not(ir), Ty::Bool))
            }
            UnOp::Neg | UnOp::Plus => {
                if !self.is_int(&ty) {
                    return Err(type_err(pos, format!("unary sign needs an int, found {}", self.tn(&ty))));
                }
                Ok((if op == UnOp::Neg { Ir::Helper(Helper::Neg, vec![ir]) } else { ir }, ty))
            }
        }
    }

    fn binary(&mut self, op: BinOp, l: &Expr, r: &Expr, pos: Pos) -> R<(Ir, Ty)> {
        match op {
            BinOp::And | BinOp::Or | BinOp::CondAnd | BinOp::CondOr => {
                let left = self.condition(l)?;
                let refined = if matches!(op, BinOp::And | BinOp::CondAnd) { self.narrowings(l)? } else { Vec::new() };
                let mark = self.cur().narrow.len();
                let lazy = matches!(op, BinOp::And | BinOp::Or) || !refined.is_empty();
                self.c().narrow.extend(refined);
                let saved = std::mem::take(&mut self.c().pre);
                let right = self.condition(r);
                let pre = std::mem::replace(&mut self.c().pre, saved);
                self.c().narrow.truncate(mark);
                let right = right?;
                let and = matches!(op, BinOp::And | BinOp::CondAnd);
                if !lazy && pre.is_empty() {
                    let o = if and { Op::And } else { Op::Or };
                    return Ok((Ir::Bin(o, Box::new(left), Box::new(right)), Ty::Bool));
                }
                if pre.is_empty() {
                    let ir = if and {
                        Ir::If(Box::new(left), Box::new(right), Box::new(Ir::Const(0)))
                    } else {
                        Ir::If(Box::new(left), Box::new(Ir::Const(1)), Box::new(right))
                    };
                    return Ok((ir, Ty::Bool));
                }
                // the right operand suspends or hoists: evaluate it conditionally
                let t = self.temp();
                self.c().pre.push(IrStmt::Set(t, left));
                let mut then = pre;
                then.push(IrStmt::Set(t, right));
                let run = if and { Ir::Local(t) } else { not(Ir::Local(t)) };
                self.c().pre.push(IrStmt::If(run, then, Vec::new()));
                Ok((Ir::Local(t), Ty::Bool))
            }
            BinOp::Pipe => {
                let vals = self.operands(&[(l, None), (r, None)])?;
                let (Ty::Actor(a), Ty::Actor(_)) = (&vals[0].1, &vals[1].1) else {
                    return Err(type_err(
                        pos,
                        "'|' and 'pipe' link actor references; use '||' for boolean OR and '|>' for function pipelines",
                    ));
                };
                let head = *a;
                let mut it = vals.into_iter();
                let (a_ir, b_ir) = (it.next().unwrap().0, it.next().unwrap().0);
                Ok((self.rt("__actor_link", vec![a_ir, b_ir]), Ty::Actor(head)))
            }
            _ => {
                let vals = self.operands(&[(l, None), (r, None)])?;
                let mut it = vals.into_iter();
                let ((a, ta), (b, tb)) = (it.next().unwrap(), it.next().unwrap());
                let both = |s: &Self, f: fn(&Self, &Ty) -> bool| f(s, &ta) && f(s, &tb);
                let mismatch = |s: &Self, what: &str| {
                    type_err(pos, format!("{what}, found {} and {}", s.tn(&ta), s.tn(&tb)))
                };
                match op {
                    BinOp::Add | BinOp::Sub | BinOp::Mul => {
                        if !both(self, Self::is_int) {
                            return Err(mismatch(self, "arithmetic needs int operands"));
                        }
                        let h = match op {
                            BinOp::Add => Helper::Add,
                            BinOp::Sub => Helper::Sub,
                            _ => Helper::Mul,
                        };
                        let ty = if ta == Ty::Raw || tb == Ty::Raw { Ty::Raw } else { Ty::Int };
                        Ok((Ir::Helper(h, vec![a, b]), ty))
                    }
                    BinOp::Div => Err(err(
                        pos,
                        "E_UNSUPPORTED",
                        "'/' on int is not supported: the result type of int / int is undecided (spec S-25)",
                    )),
                    BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                        if !both(self, Self::is_int) {
                            return Err(mismatch(self, "ordering needs int operands"));
                        }
                        let o = match op {
                            BinOp::Lt => Op::LtS,
                            BinOp::Le => Op::LeS,
                            BinOp::Gt => Op::GtS,
                            _ => Op::GeS,
                        };
                        Ok((Ir::Bin(o, Box::new(a), Box::new(b)), Ty::Bool))
                    }
                    BinOp::Eq | BinOp::Ne => {
                        let ir = if ta == Ty::Str && tb == Ty::Str {
                            let same = self.rt("__str_eq", vec![a, b]);
                            if op == BinOp::Eq {
                                Ir::Bin(Op::Ne, Box::new(same), Box::new(Ir::Const(0)))
                            } else {
                                not(same)
                            }
                        } else if both(self, Self::is_int) || both(self, Self::is_bool) {
                            Ir::Bin(if op == BinOp::Eq { Op::Eq } else { Op::Ne }, Box::new(a), Box::new(b))
                        } else {
                            return Err(mismatch(self, "equality compares two ints, two bools or two strings"));
                        };
                        Ok((ir, Ty::Bool))
                    }
                    BinOp::Xor => {
                        if !both(self, Self::is_bool) {
                            return Err(mismatch(self, "'^^' needs bool operands"));
                        }
                        Ok((Ir::Bin(Op::Xor, Box::new(a), Box::new(b)), Ty::Bool))
                    }
                    _ => unreachable!(),
                }
            }
        }
    }

    fn member(&mut self, base: &Expr, name: &str, pos: Pos) -> R<(Ir, Ty)> {
        if let Ex::Var(n) = &base.kind {
            if let Some(v) = self.find(n, base.pos)? {
                if let Ty::Mail(payload) = &v.ty {
                    return match name {
                        "value" => Ok((Ir::Local(v.slot), (**payload).clone())),
                        "sequence" => Ok((Ir::Local(v.slot + 1), Ty::Int)),
                        _ => Err(type_err(pos, "ActorMail has 'value' and 'sequence'")),
                    };
                }
            }
        }
        if matches!(base.kind, Ex::SelfRef) {
            return match self.self_field(name, pos)? {
                Some((addr, f)) => Ok((Ir::Load(Box::new(addr)), f.ty)),
                None => Err(err(pos, "E_UNDEFINED", format!("unknown field '{name}'"))),
            };
        }
        let (ir, ty) = self.value(base, None)?;
        match &ty {
            Ty::Object(i) => match self.objs[*i].fields.iter().find(|f| f.name == name).cloned() {
                Some(f) => Ok((load(ir, f.offset as i64), f.ty)),
                None => Err(err(pos, "E_UNDEFINED", format!("{} has no field '{name}'", self.tn(&ty)))),
            },
            _ => Err(type_err(pos, format!("{} has no member '{name}'", self.tn(&ty)))),
        }
    }

    fn await_expr(&mut self, inner: &Expr, pos: Pos) -> R<(Ir, Ty)> {
        self.need_suspends(pos, "'await'")?;
        let (ir, ty, direct) = match &inner.kind {
            Ex::Call { name, targs, args } => self.call(name, targs, args, None, inner.pos, true)?,
            Ex::Method { base, name, args } => self.method(base, name, args, inner.pos, true)?,
            _ => {
                let (ir, ty) = self.expr(inner, None)?;
                (ir, ty, false)
            }
        };
        if direct {
            return Ok((ir, ty));
        }
        match ty {
            Ty::Future(t) => Ok((self.rt_scall("__await", vec![ir]), *t)),
            Ty::Raw => Ok((self.rt_scall("__await", vec![ir]), Ty::Raw)),
            other => Err(err(
                pos,
                "E_AWAIT",
                format!("'await' needs a Future, a Task or a call of a suspends function, found {}", self.tn(&other)),
            )),
        }
    }

    fn plain_args<'b>(&self, callee: &str, args: &'b [Arg], n: usize, pos: Pos) -> R<Vec<&'b Expr>> {
        if let Some(a) = args.iter().find(|a| a.name.is_some()) {
            return Err(unsupported(a.value.pos, "named arguments here"));
        }
        if args.len() != n {
            return Err(err(pos, "E_ARITY", format!("'{callee}' takes {n} argument(s), {} given", args.len())));
        }
        Ok(args.iter().map(|a| &a.value).collect())
    }

    /// Call `id` with `first` prepended to the checked arguments.
    fn invoke(&mut self, callee: &str, id: u32, first: Option<Ir>, args: &[Arg], pos: Pos, awaited: bool) -> R<(Ir, Ty, bool)> {
        let sig = self.sigs[id as usize].clone();
        let exprs = self.plain_args(callee, args, sig.params.len(), pos)?;
        let pairs: Vec<(&Expr, Option<Ty>)> = exprs.iter().map(|e| *e).zip(sig.params.iter().cloned().map(Some)).collect();
        let vals = self.operands(&pairs)?;
        let mut irs: Vec<Ir> = first.into_iter().collect();
        for (((ir, ty), want), e) in vals.into_iter().zip(&sig.params).zip(&exprs) {
            irs.push(self.coerce(ir, &ty, want, e.pos)?);
        }
        if sig.suspends && sig.async_gen.is_none() {
            if !awaited {
                return Err(err(pos, "E_AWAIT", format!("'{callee}' is a suspends function; call it with 'await {callee}(...)'")));
            }
            self.need_suspends(pos, "calling a suspends function")?;
            return Ok((self.scall(id, irs), sig.ret, true));
        }
        if sig.async_gen.is_some() {
            self.needs_sched = true;
        }
        Ok((Ir::Call(id, irs), sig.ret, false))
    }

    #[allow(clippy::too_many_arguments)]
    fn call(&mut self, name: &str, targs: &[TyEx], args: &[Arg], want: Option<&Ty>, pos: Pos, awaited: bool) -> R<(Ir, Ty, bool)> {
        if name == "channel" && targs.len() == 1 {
            let item = self.resolve(&targs[0], false)?;
            if args.len() != 1 || args[0].name.as_deref().is_some_and(|n| n != "capacity") {
                return Err(type_err(pos, "a channel is created with channel<T>(capacity: n)"));
            }
            let n = self.typed(&args[0].value, &Ty::Int)?;
            return Ok((self.rt("__chan_new", vec![n]), Ty::Channel(Box::new(item)), false));
        }
        if !targs.is_empty() {
            return Err(unsupported(pos, "explicit type arguments on this call"));
        }
        if let Some(v) = self.find(name, pos)? {
            let Ty::Fn(params, ret) = v.ty.clone() else {
                return Err(type_err(pos, format!("'{name}' is a {}, not a function", self.tn(&v.ty))));
            };
            let exprs = self.plain_args(name, args, params.len(), pos)?;
            let pairs: Vec<(&Expr, Option<Ty>)> = exprs.iter().map(|e| *e).zip(params.iter().cloned().map(Some)).collect();
            let vals = self.operands(&pairs)?;
            let mut irs = Vec::new();
            for (((ir, ty), w), e) in vals.into_iter().zip(&params).zip(&exprs) {
                irs.push(self.coerce(ir, &ty, w, e.pos)?);
            }
            return Ok((Ir::CallClosure(Box::new(Ir::Local(v.slot)), irs), *ret, false));
        }
        if self.cur().prelude && name.starts_with("__") {
            return self.intrinsic(name, args, pos, awaited);
        }
        match name {
            "Some" => {
                let inner = match want {
                    Some(Ty::Option(t)) if !t.has_hole() => Some((**t).clone()),
                    _ => None,
                };
                let e = self.plain_args(name, args, 1, pos)?[0];
                let (ir, ty) = self.value(e, inner.as_ref())?;
                let (ir, ty) = match &inner {
                    Some(w) => (self.coerce(ir, &ty, w, e.pos)?, w.clone()),
                    None => (ir, ty),
                };
                return Ok((self.rt("__some", vec![ir]), Ty::Option(Box::new(ty)), false));
            }
            "Ok" | "Err" => {
                let (ok, bad) = match want {
                    Some(Ty::Result(a, b)) => ((**a).clone(), (**b).clone()),
                    _ => (Ty::Infer, Ty::Infer),
                };
                let side = if name == "Ok" { &ok } else { &bad };
                let e = self.plain_args(name, args, 1, pos)?[0];
                let hint = if side.has_hole() { None } else { Some(side.clone()) };
                let (ir, ty) = self.value(e, hint.as_ref())?;
                let (ir, ty) = match &hint {
                    Some(w) => (self.coerce(ir, &ty, w, e.pos)?, w.clone()),
                    None => (ir, ty),
                };
                let (tag, res) = if name == "Ok" {
                    (0, Ty::Result(Box::new(ty), Box::new(bad)))
                } else {
                    (1, Ty::Result(Box::new(ok), Box::new(ty)))
                };
                return Ok((self.rt("__box", vec![Ir::Const(tag), ir]), res, false));
            }
            "And" | "Or" | "Xor" => return self.gate(name, args, pos).map(|ir| (ir, Ty::Bool, false)),
            "emit" => {
                let Some(this) = self.find("self", pos)? else {
                    return Err(err(pos, "E_ACTOR", "'emit' is only valid inside an actor"));
                };
                let Ty::Actor(i) = this.ty else {
                    return Err(err(pos, "E_ACTOR", "'emit' is only valid inside an actor"));
                };
                let e = self.plain_args(name, args, 1, pos)?[0];
                let known = self.objs[i].emit.clone();
                let (ir, ty) = self.value(e, known.as_ref())?;
                match &known {
                    Some(k) if !fits(&ty, k) => {
                        return Err(type_err(e.pos, format!("this actor emits {}, found {}", self.tn(k), self.tn(&ty))));
                    }
                    Some(_) => {}
                    None => self.objs[i].emit = Some(ty),
                }
                let call = self.rt("__actor_emit", vec![Ir::Local(this.slot), ir]);
                self.c().pre.push(IrStmt::Expr(call));
                return Ok((Ir::Const(0), Ty::Void, false));
            }
            "timeout" => return Err(type_err(pos, "timeout(ms) is a scope: write 'with timeout(ms) { ... }'")),
            _ => {}
        }
        if let Some(&id) = self.user.get(name) {
            return self.invoke(name, id, None, args, pos, awaited);
        }
        if let Some(i) = self.objs.iter().position(|o| o.name == name) {
            if self.objs[i].kind == ObjKind::Actor {
                return Err(err(pos, "E_ACTOR", format!("start an actor with 'spawn {name}()'")));
            }
            self.plain_args(name, args, 0, pos)?;
            return Ok((Ir::Call(self.objs[i].ctor, Vec::new()), Ty::Object(i), false));
        }
        if let Some(this) = self.find("self", pos)? {
            if let Ty::Actor(i) | Ty::Object(i) = this.ty {
                if let Some(&(_, id)) = self.objs[i].methods.iter().find(|(n, _)| n == name) {
                    return self.invoke(name, id, Some(Ir::Local(this.slot)), args, pos, awaited);
                }
            }
        }
        Err(err(pos, "E_UNDEFINED", format!("unknown function '{name}'")))
    }

    /// `And`/`Or`/`Xor` over two or more bools, or over one `List<bool>`.
    fn gate(&mut self, name: &str, args: &[Arg], pos: Pos) -> R<Ir> {
        if let Some(a) = args.iter().find(|a| a.name.is_some()) {
            return Err(unsupported(a.value.pos, "named arguments here"));
        }
        let kind = match name {
            "And" => 0,
            "Or" => 1,
            _ => 2,
        };
        if args.len() == 1 {
            let (ir, ty) = self.value(&args[0].value, Some(&Ty::List(Box::new(Ty::Bool))))?;
            if ty != Ty::List(Box::new(Ty::Bool)) {
                return Err(type_err(pos, format!("'{name}' takes two or more bools or one list of bools, found {}", self.tn(&ty))));
            }
            return Ok(self.rt("__gate", vec![ir, Ir::Const(kind)]));
        }
        if args.len() < 2 {
            return Err(err(pos, "E_ARITY", format!("'{name}' takes two or more bools or one list of bools")));
        }
        let pairs: Vec<(&Expr, Option<Ty>)> = args.iter().map(|a| (&a.value, Some(Ty::Bool))).collect();
        let vals = self.operands(&pairs)?;
        let mut acc: Option<Ir> = None;
        for ((ir, ty), a) in vals.into_iter().zip(args) {
            if !self.is_bool(&ty) {
                return Err(type_err(a.value.pos, format!("'{name}' needs bool arguments, found {}", self.tn(&ty))));
            }
            let op = [Op::And, Op::Or, Op::Xor][kind as usize];
            acc = Some(match acc {
                None => ir,
                Some(prev) => Ir::Bin(op, Box::new(prev), Box::new(ir)),
            });
        }
        Ok(acc.unwrap())
    }

    /// Runtime-only primitives available to the prelude.
    fn intrinsic(&mut self, name: &str, args: &[Arg], pos: Pos, awaited: bool) -> R<(Ir, Ty, bool)> {
        let pairs: Vec<(&Expr, Option<Ty>)> = args.iter().map(|a| (&a.value, None)).collect();
        let arity = |n: usize| -> R<()> {
            if args.len() == n {
                Ok(())
            } else {
                Err(err(pos, "E_ARITY", format!("'{name}' takes {n} argument(s)")))
            }
        };
        let konst = |ir: &Ir| -> R<i64> {
            match ir {
                Ir::Const(v) => Ok(*v),
                _ => Err(err(pos, "E_TYPE", format!("'{name}' needs a constant here"))),
            }
        };
        let addr = |p: Ir, off: Ir| match off {
            Ir::Const(0) => p,
            off => Ir::Bin(Op::AddRaw, Box::new(p), Box::new(off)),
        };
        let void = (Ir::Const(0), Ty::Void, false);
        let fixed: Option<usize> = match name {
            "__state" | "__fsp" | "__suspend" => Some(0),
            "__alloc" | "__load8" | "__rt" | "__set_state" | "__set_fsp" | "__set_fend" | "__call0" | "__panic" => Some(1),
            "__load" | "__store8" | "__set_rt" | "__call1" | "__and" | "__or" | "__shr" | "__shl" => Some(2),
            "__store" => Some(3),
            _ => None,
        };
        let Some(n) = fixed else {
            let Some(idx) = self.prelude.funcs.iter().position(|f| f.name == name) else {
                return Err(err(pos, "E_UNDEFINED", format!("unknown runtime function '{name}'")));
            };
            let _ = idx;
            let id = self.rt_id(name);
            return self.invoke(name, id, None, args, pos, awaited);
        };
        arity(n)?;
        let mut v: Vec<Ir> = self.operands(&pairs)?.into_iter().map(|x| x.0).collect();
        let mut next = || v.remove(0);
        Ok(match name {
            "__alloc" => (Ir::Helper(Helper::Alloc, vec![next()]), Ty::Raw, false),
            "__load" => {
                let (p, off) = (next(), next());
                (Ir::Load(Box::new(addr(p, off))), Ty::Raw, false)
            }
            "__load8" => (Ir::Load8(Box::new(next())), Ty::Raw, false),
            "__store" => {
                let (p, off, val) = (next(), next(), next());
                self.c().pre.push(IrStmt::Store(addr(p, off), val));
                void
            }
            "__store8" => {
                let (p, val) = (next(), next());
                self.c().pre.push(IrStmt::Store8(p, val));
                void
            }
            "__rt" => (Ir::Load(Box::new(Ir::Const(8 * konst(&next())?))), Ty::Raw, false),
            "__set_rt" => {
                let (slot, val) = (next(), next());
                self.c().pre.push(IrStmt::Store(Ir::Const(8 * konst(&slot)?), val));
                void
            }
            "__state" => (Ir::Global(G_STATE), Ty::Raw, false),
            "__fsp" => (Ir::Global(G_FSP), Ty::Raw, false),
            "__set_state" | "__set_fsp" | "__set_fend" => {
                let g = match name {
                    "__set_state" => G_STATE,
                    "__set_fsp" => G_FSP,
                    _ => G_FEND,
                };
                let val = next();
                self.c().pre.push(IrStmt::GlobalSet(g, val));
                void
            }
            "__call0" | "__call1" => {
                let clo = next();
                let clo = self.spill(clo);
                let rest: Vec<Ir> = v.drain(..).collect();
                (Ir::CallClosure(Box::new(clo), rest), Ty::Raw, false)
            }
            "__suspend" => {
                let c = self.c();
                c.pre.push(IrStmt::Suspend);
                c.has_susp = true;
                void
            }
            "__panic" => {
                let code = konst(&next())? as i32;
                self.c().pre.push(IrStmt::Panic(code));
                (Ir::Const(0), Ty::Never, false)
            }
            "__and" | "__or" | "__shr" | "__shl" => {
                let op = match name {
                    "__and" => Op::And,
                    "__or" => Op::Or,
                    "__shr" => Op::ShrU,
                    _ => Op::Shl,
                };
                let (a, b) = (next(), next());
                (Ir::Bin(op, Box::new(a), Box::new(b)), Ty::Raw, false)
            }
            _ => unreachable!(),
        })
    }

    fn print(&mut self, newline: bool, args: &[Arg], pos: Pos) -> R<(Ir, Ty, bool)> {
        if let Some(a) = args.iter().find(|a| a.name.is_some()) {
            return Err(unsupported(a.value.pos, "named arguments here"));
        }
        if args.len() > 1 || (args.is_empty() && !newline) {
            return Err(err(pos, "E_ARITY", "stdio.print/println take one value"));
        }
        if let Some(a) = args.first() {
            let (ir, ty) = self.value(&a.value, None)?;
            let (kind, ir) = match ty {
                Ty::Int | Ty::Raw => (Print::I64, ir),
                Ty::Bool => (Print::Bool, ir),
                Ty::Str => (Print::Str, self.spill(ir)),
                other => {
                    return Err(type_err(a.value.pos, format!("stdio can print int, bool and string values, not {}", self.tn(&other))));
                }
            };
            self.uses_print[kind as usize] = true;
            self.c().pre.push(IrStmt::Print(kind, Some(ir)));
        }
        if newline {
            self.uses_print[Print::Newline as usize] = true;
            self.c().pre.push(IrStmt::Print(Print::Newline, None));
        }
        Ok((Ir::Const(0), Ty::Void, false))
    }

    fn method(&mut self, base: &Expr, name: &str, args: &[Arg], pos: Pos, awaited: bool) -> R<(Ir, Ty, bool)> {
        // module namespaces, unless shadowed by a variable or field
        if let Ex::Var(ns) = &base.kind {
            let shadowed = self.find(ns, base.pos)?.is_some() || self.self_field(ns, base.pos)?.is_some();
            if !shadowed {
                match (ns.as_str(), name) {
                    ("stdio", "print") => return self.print(false, args, pos),
                    ("stdio", "println") => return self.print(true, args, pos),
                    ("stdio", _) => return Err(unsupported(pos, "stdio members other than print and println")),
                    ("time", "sleep") if self.has_time => {
                        if !awaited {
                            return Err(err(pos, "E_AWAIT", "time.sleep suspends; call it with 'await time.sleep(ms)'"));
                        }
                        let e = self.plain_args("time.sleep", args, 1, pos)?[0];
                        let ms = self.typed(e, &Ty::Int)?;
                        self.rt_scall("__sleep", vec![ms]);
                        return Ok((Ir::Const(0), Ty::Void, true));
                    }
                    ("telemetry", "span") if self.has_telemetry => {
                        let e = self.plain_args("telemetry.span", args, 1, pos)?[0];
                        let n = self.typed(e, &Ty::Str)?;
                        return Ok((self.rt("__span_new", vec![n]), Ty::Span, false));
                    }
                    ("time" | "telemetry", _) if !self.has_time && !self.has_telemetry => {
                        return Err(err(pos, "E_MODULE", format!("'{ns}' needs a 'use' declaration (std.time, core.telemetry)")));
                    }
                    ("rx", "from_channel") => {
                        let e = self.plain_args("rx.from_channel", args, 1, pos)?[0];
                        let (ir, ty) = self.value(e, None)?;
                        let Ty::Channel(item) = ty else {
                            return Err(type_err(e.pos, format!("rx.from_channel needs a Channel<T>, found {}", self.tn(&ty))));
                        };
                        let node = self.rt("__rx_node", vec![Ir::Const(4), Ir::Const(0), ir, Ir::Const(0)]);
                        return Ok((node, Ty::Observable(item), false));
                    }
                    _ => {}
                }
            }
        }
        if let Ex::Member(inner, member) = &base.kind {
            if matches!(&inner.kind, Ex::Var(n) if n == "rx") && member == "Observable" && name == "from_iter" {
                let e = self.plain_args("rx.Observable.from_iter", args, 1, pos)?[0];
                let (ir, ty) = self.value(e, None)?;
                let Ty::List(item) = ty else {
                    return Err(type_err(e.pos, format!("from_iter needs a list, found {}", self.tn(&ty))));
                };
                let node = self.rt("__rx_node", vec![Ir::Const(0), Ir::Const(0), ir, Ir::Const(0)]);
                return Ok((node, Ty::Observable(item), false));
            }
        }
        let (recv, ty) = self.value(base, None)?;
        let recv = self.spill(recv);
        match &ty {
            Ty::Channel(item) => match name {
                "close" => {
                    self.plain_args(name, args, 0, pos)?;
                    let call = self.rt("__chan_close", vec![recv]);
                    self.c().pre.push(IrStmt::Expr(call));
                    Ok((Ir::Const(0), Ty::Void, false))
                }
                "send" => {
                    self.need_suspends(pos, "sending on a channel")?;
                    let e = self.plain_args(name, args, 1, pos)?[0];
                    let v = self.typed(e, item)?;
                    self.rt_scall("__chan_send", vec![recv, v]);
                    Ok((Ir::Const(0), Ty::Void, true))
                }
                "receive" => {
                    if !awaited {
                        return Err(err(pos, "E_AWAIT", "receive suspends; call it with 'await ch.receive()'"));
                    }
                    self.plain_args(name, args, 0, pos)?;
                    Ok((self.rt_scall("__chan_recv", vec![recv]), (**item).clone(), true))
                }
                _ => Err(type_err(pos, format!("a channel has close(), send(v) and receive(), not '{name}'"))),
            },
            Ty::Actor(i) => {
                if name == "send" {
                    let mail = self.objs[*i].mail.clone();
                    if mail == Ty::Void {
                        return Err(err(pos, "E_ACTOR", format!("actor '{}' has no receive handler", self.objs[*i].name)));
                    }
                    let e = self.plain_args(name, args, 1, pos)?[0];
                    let v = self.typed(e, &mail)?;
                    let call = self.rt("__mail_send", vec![recv, v]);
                    self.c().pre.push(IrStmt::Expr(call));
                    return Ok((Ir::Const(0), Ty::Void, false));
                }
                Err(err(pos, "E_ACTOR", "an actor reference only accepts send(value); actor methods are private"))
            }
            Ty::Object(i) => match self.objs[*i].methods.iter().find(|(n, _)| n == name) {
                Some(&(_, id)) => self.invoke(name, id, Some(recv), args, pos, awaited),
                None => Err(err(pos, "E_UNDEFINED", format!("{} has no method '{name}'", self.tn(&ty)))),
            },
            Ty::Observable(item) => {
                let item = (**item).clone();
                match name {
                    "filter" | "map" | "subscribe" => {
                        let e = self.plain_args(name, args, 1, pos)?[0];
                        let ret = match name {
                            "filter" => Ty::Bool,
                            "map" => Ty::Infer,
                            _ => Ty::Void,
                        };
                        let want = Ty::Fn(vec![item.clone()], Box::new(ret.clone()));
                        let (f, fty) = self.value(e, Some(&want))?;
                        let Ty::Fn(p, r) = &fty else {
                            return Err(type_err(e.pos, format!("'{name}' needs a function, found {}", self.tn(&fty))));
                        };
                        if p.len() != 1 || !fits(&item, &p[0]) || !fits(r, &ret) || (name == "map" && **r == Ty::Void) {
                            return Err(type_err(e.pos, format!("'{name}' needs {}, found {}", self.tn(&want), self.tn(&fty))));
                        }
                        match name {
                            "filter" => Ok((self.rt("__rx_node", vec![Ir::Const(2), recv, f, Ir::Const(0)]), ty.clone(), false)),
                            "map" => {
                                let out = Ty::Observable(r.clone());
                                Ok((self.rt("__rx_node", vec![Ir::Const(1), recv, f, Ir::Const(0)]), out, false))
                            }
                            _ => Ok((self.rt("__rx_subscribe", vec![recv, f]), Ty::Subscription, false)),
                        }
                    }
                    "take" => {
                        let e = self.plain_args(name, args, 1, pos)?[0];
                        let n = self.typed(e, &Ty::Int)?;
                        Ok((self.rt("__rx_node", vec![Ir::Const(3), recv, Ir::Const(0), n]), ty.clone(), false))
                    }
                    "first_task" => {
                        self.plain_args(name, args, 0, pos)?;
                        let fut = Ty::Future(Box::new(Ty::Option(Box::new(item))));
                        Ok((self.rt("__rx_first_task", vec![recv]), fut, false))
                    }
                    _ => Err(type_err(pos, format!("an Observable has filter, map, take, subscribe and first_task, not '{name}'"))),
                }
            }
            Ty::Subscription => match name {
                "completed_task" => {
                    self.plain_args(name, args, 0, pos)?;
                    Ok((load(recv, 0), Ty::Future(Box::new(Ty::Void)), false))
                }
                "dispose" => {
                    self.plain_args(name, args, 0, pos)?;
                    let call = self.rt("__sub_dispose", vec![recv]);
                    self.c().pre.push(IrStmt::Expr(call));
                    Ok((Ir::Const(0), Ty::Void, false))
                }
                _ => Err(type_err(pos, format!("a Subscription has completed_task() and dispose(), not '{name}'"))),
            },
            Ty::Span => match name {
                "set_attribute" => {
                    let e = self.plain_args(name, args, 2, pos)?;
                    let vals = self.operands(&[(e[0], Some(Ty::Str)), (e[1], Some(Ty::Str))])?;
                    if vals.iter().any(|(_, t)| *t != Ty::Str) {
                        return Err(type_err(pos, "set_attribute takes a string key and a string value"));
                    }
                    let mut it = vals.into_iter();
                    let call = self.rt("__span_attr", vec![recv, it.next().unwrap().0, it.next().unwrap().0]);
                    self.c().pre.push(IrStmt::Expr(call));
                    Ok((Ir::Const(0), Ty::Void, false))
                }
                _ => Err(type_err(pos, format!("a Span has set_attribute(key, value), not '{name}'"))),
            },
            other => Err(type_err(pos, format!("{} has no method '{name}'", self.tn(other)))),
        }
    }

    // ------------------------------------------------------------- match

    fn match_expr(&mut self, subject: &Expr, arms: &[Arm], want: Option<&Ty>, pos: Pos) -> R<(Ir, Ty)> {
        // `on is T` refines the subject when it is a plain `Any` variable
        let refine_slot = match &subject.kind {
            Ex::Var(n) => self.find(n, subject.pos)?.filter(|v| v.ty == Ty::Any).map(|v| v.slot),
            _ => None,
        };
        let (s_ir, s_ty) = match refine_slot {
            Some(slot) => (Ir::Local(slot), Ty::Any),
            None => self.value(subject, None)?,
        };
        let s = self.temp();
        self.c().pre.push(IrStmt::Set(s, s_ir));
        let matched = self.temp();
        self.c().pre.push(IrStmt::Set(matched, Ir::Const(0)));
        let result = self.temp();
        let subj = Ir::Local(s);

        let mut result_ty = Ty::Never;
        let mut all_diverge = true;
        let (mut wild, mut seen) = (false, Vec::<String>::new());
        for arm in arms {
            self.c().scopes.push(Vec::new());
            let mark = self.cur().narrow.len();
            let mut binds = Vec::new();
            let mut covers: Option<String> = None;
            let test = match &arm.pat {
                Pat::Wild => {
                    covers = Some("_".into());
                    Ir::Const(1)
                }
                Pat::Bind(name) => {
                    let slot = self.declare(name, s_ty.clone(), false, arm.pos)?;
                    binds.push(IrStmt::Set(slot, subj.clone()));
                    covers = Some("_".into());
                    Ir::Const(1)
                }
                Pat::Int(v) => {
                    if !self.is_int(&s_ty) {
                        return Err(type_err(arm.pos, format!("an integer pattern cannot match {}", self.tn(&s_ty))));
                    }
                    eq(subj.clone(), Ir::Const(*v))
                }
                Pat::Bool(b) => {
                    if !self.is_bool(&s_ty) {
                        return Err(type_err(arm.pos, format!("a bool pattern cannot match {}", self.tn(&s_ty))));
                    }
                    covers = Some(b.to_string());
                    eq(subj.clone(), Ir::Const(*b as i64))
                }
                Pat::Str(text) => {
                    if s_ty != Ty::Str {
                        return Err(type_err(arm.pos, format!("a string pattern cannot match {}", self.tn(&s_ty))));
                    }
                    let lit = self.string(text, arm.pos)?;
                    let same = self.rt("__str_eq", vec![subj.clone(), lit]);
                    Ir::Bin(Op::Ne, Box::new(same), Box::new(Ir::Const(0)))
                }
                Pat::Is(t) => {
                    if s_ty != Ty::Any {
                        return Err(type_err(arm.pos, format!("'on is T' matches an Any value, found {}", self.tn(&s_ty))));
                    }
                    let target = self.resolve(t, false)?;
                    let tag = self.tag_of(&target, t.pos())?;
                    if let Some(slot) = refine_slot {
                        self.c().narrow.push((slot, target));
                    }
                    eq(load(subj.clone(), 0), Ir::Const(tag))
                }
                Pat::Variant(variant, payload) => {
                    covers = Some(variant.clone());
                    let (test, value, ty) = match (&s_ty, variant.as_str()) {
                        (Ty::Option(t), "Some") => {
                            (Ir::Bin(Op::Ne, Box::new(subj.clone()), Box::new(Ir::Const(0))), load(subj.clone(), 0), (**t).clone())
                        }
                        (Ty::Option(_), "None") => (eq(subj.clone(), Ir::Const(0)), Ir::Const(0), Ty::Void),
                        (Ty::Result(t, _), "Ok") => (eq(load(subj.clone(), 0), Ir::Const(0)), load(subj.clone(), 8), (**t).clone()),
                        (Ty::Result(_, e), "Err") => (eq(load(subj.clone(), 0), Ir::Const(1)), load(subj.clone(), 8), (**e).clone()),
                        _ => {
                            return Err(type_err(arm.pos, format!("pattern '{variant}' cannot match {}", self.tn(&s_ty))));
                        }
                    };
                    if let Some(name) = payload {
                        if name != "_" {
                            let slot = self.declare(name, ty, false, arm.pos)?;
                            binds.push(IrStmt::Set(slot, value));
                        }
                    }
                    test
                }
            };
            let guard = match &arm.guard {
                Some(g) => {
                    let (pre, ir, ty) = self.isolated(g, Some(&Ty::Bool))?;
                    if !self.is_bool(&ty) {
                        return Err(type_err(g.pos, format!("a guard is a bool, found {}", self.tn(&ty))));
                    }
                    Some((pre, ir))
                }
                None => None,
            };
            let mut body = vec![IrStmt::Set(matched, Ir::Const(1))];
            let arm_ty = match &arm.body {
                ArmBody::Block(stmts) => {
                    let (b, d) = self.stmts(stmts)?;
                    body.extend(b);
                    if d {
                        Ty::Never
                    } else {
                        Ty::Void
                    }
                }
                ArmBody::Expr(e) => {
                    let hint = want.cloned().or_else(|| if result_ty.has_hole() || result_ty == Ty::Never { None } else { Some(result_ty.clone()) });
                    let (pre, ir, ty) = self.isolated(e, hint.as_ref())?;
                    body.extend(pre);
                    if ty != Ty::Void && ty != Ty::Never {
                        let ir = match want {
                            Some(w) if *w == Ty::Any => self.coerce(ir, &ty, w, e.pos)?,
                            _ => ir,
                        };
                        body.push(IrStmt::Set(result, ir));
                    } else if !simple(&ir) {
                        body.push(IrStmt::Expr(ir));
                    }
                    if matches!(want, Some(Ty::Any)) && ty != Ty::Never {
                        Ty::Any
                    } else {
                        ty
                    }
                }
            };
            self.c().narrow.truncate(mark);
            self.c().scopes.pop();
            if arm_ty != Ty::Never {
                all_diverge = false;
                if result_ty == Ty::Never {
                    result_ty = arm_ty;
                } else if !fits(&arm_ty, &result_ty) || (arm_ty == Ty::Void) != (result_ty == Ty::Void) {
                    return Err(type_err(arm.pos, format!("match arms differ: {} and {}", self.tn(&result_ty), self.tn(&arm_ty))));
                } else {
                    result_ty = merge(&result_ty, &arm_ty);
                }
            }
            let mut inner = binds;
            match guard {
                Some((pre, g)) => {
                    inner.extend(pre);
                    inner.push(IrStmt::If(g, body, Vec::new()));
                }
                None => {
                    inner.extend(body);
                    if let Some(c) = covers {
                        if c == "_" {
                            wild = true;
                        }
                        seen.push(c);
                    }
                }
            }
            let cond = match test {
                Ir::Const(1) => not(Ir::Local(matched)),
                test => Ir::If(Box::new(Ir::Local(matched)), Box::new(Ir::Const(0)), Box::new(test)),
            };
            self.c().pre.push(IrStmt::If(cond, inner, Vec::new()));
        }
        let has = |n: &str| seen.iter().any(|s| s == n);
        let exhaustive = wild
            || match &s_ty {
                Ty::Option(_) => has("Some") && has("None"),
                Ty::Result(..) => has("Ok") && has("Err"),
                Ty::Bool => has("true") && has("false"),
                _ => false,
            };
        if !exhaustive {
            return Err(err(pos, "E_MATCH", "match is not exhaustive; add an 'on _ -> ...' or 'default -> ...' arm"));
        }
        if all_diverge {
            return Ok((Ir::Const(0), Ty::Never));
        }
        if result_ty == Ty::Void {
            return Ok((Ir::Const(0), Ty::Void));
        }
        Ok((Ir::Local(result), result_ty))
    }
}

/// Move a context out through a `&mut` (the borrow checker's way to push a
/// freshly built context that the caller no longer needs).
fn cx_take(cx: &mut FnCx) -> FnCx {
    std::mem::replace(cx, FnCx::new(Ty::Void, false, false))
}
