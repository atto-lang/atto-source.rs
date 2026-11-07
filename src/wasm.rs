//! Typed IR -> WebAssembly binary (core spec 1.0 plus mutable-global export).
//!
//! Guest contract `atto.browser.guest.v1`:
//! * export `main: () -> i64` (a void `main` returns 0)
//! * export `panic_code: mut i32`, set before a language-level panic traps:
//!   1 = integer overflow; code 2 is reserved (division/remainder unsupported)
//! * export `memory` only when string literals are printed
//! * imports, only those used, from module `atto`:
//!   `print_i64(i64)`, `print_bool(i32)`, `print_str(i32 offset, i32 len)`,
//!   `print_newline()`
//!
//! Output is a pure function of the source: no hash-map iteration, clocks or
//! randomness, so identical input yields byte-identical modules on every host.

use crate::ast::{BinOp, Type};
use crate::check::{Builtin, Import, TExpr, TProgram, TStmt, TE};

pub const MAX_OUTPUT: usize = 1 << 20;
pub const PANIC_OVERFLOW: i32 = 1;
/// Reserved ABI value; current admitted expressions cannot generate this panic.
pub const PANIC_DIV_ZERO: i32 = 2;

const I32: u8 = 0x7F;
const I64: u8 = 0x7E;

fn uleb(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let b = (v & 0x7F) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            break;
        }
        out.push(b | 0x80);
    }
}

fn sleb(out: &mut Vec<u8>, mut v: i64) {
    loop {
        let b = (v & 0x7F) as u8;
        v >>= 7;
        let done = (v == 0 && b & 0x40 == 0) || (v == -1 && b & 0x40 != 0);
        out.push(if done { b } else { b | 0x80 });
        if done {
            break;
        }
    }
}

fn name(out: &mut Vec<u8>, s: &str) {
    uleb(out, s.len() as u64);
    out.extend_from_slice(s.as_bytes());
}

fn section(out: &mut Vec<u8>, id: u8, body: Vec<u8>) {
    out.push(id);
    uleb(out, body.len() as u64);
    out.extend(body);
}

fn vt(t: Type) -> Option<u8> {
    match t {
        Type::Int | Type::Enum(_) => Some(I64),
        Type::Bool => Some(I32),
        Type::Class(_) => Some(I32),
        Type::Void => None,
    }
}

#[derive(Clone, PartialEq, Eq)]
struct Sig(Vec<u8>, Vec<u8>);

struct Types(Vec<Sig>);
impl Types {
    fn idx(&mut self, s: Sig) -> u32 {
        if let Some(i) = self.0.iter().position(|x| *x == s) {
            return i as u32;
        }
        self.0.push(s);
        (self.0.len() - 1) as u32
    }
}

// helper function order, placed right after imports
const H_ADD: u32 = 0;
const H_SUB: u32 = 1;
const H_MUL: u32 = 2;
const H_NEG: u32 = 3;
const HELPERS: u32 = 4;

struct Layout {
    imports: Vec<Import>,
}
impl Layout {
    fn import(&self, i: Import) -> u32 {
        self.imports.iter().position(|x| *x == i).expect("import registered by checker") as u32
    }
    fn helper(&self, h: u32) -> u32 {
        self.imports.len() as u32 + h
    }
    fn func(&self, f: u32) -> u32 {
        self.imports.len() as u32 + HELPERS + f
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Frame {
    Break,
    Top,
    Continue,
    Other,
}

struct Emit<'a> {
    l: &'a Layout,
    code: Vec<u8>,
    frames: Vec<Frame>,
    /// `Some(..)` when compiling a void `main` export: plain `return` must
    /// still produce the exported i64 status 0.
    void_main: bool,
    /// Per-call i32 temporary for freshly allocated nonescaping object.
    object_tmp: Option<u32>,
}

impl Emit<'_> {
    fn op(&mut self, b: u8) {
        self.code.push(b);
    }
    fn i64c(&mut self, v: i64) {
        self.op(0x42);
        sleb(&mut self.code, v);
    }
    fn i32c(&mut self, v: i32) {
        self.op(0x41);
        sleb(&mut self.code, v as i64);
    }
    fn call(&mut self, idx: u32) {
        self.op(0x10);
        uleb(&mut self.code, idx as u64);
    }
    fn br_to(&mut self, f: Frame) {
        let at = self.frames.iter().rposition(|x| *x == f).expect("loop frame");
        self.op(0x0C);
        uleb(&mut self.code, (self.frames.len() - 1 - at) as u64);
    }
    fn open(&mut self, opcode: u8, blocktype: u8, f: Frame) {
        self.op(opcode);
        self.op(blocktype);
        self.frames.push(f);
    }
    fn close(&mut self) {
        self.op(0x0B);
        self.frames.pop();
    }

    fn stmts(&mut self, s: &[TStmt]) {
        for st in s {
            self.stmt(st);
        }
    }

    fn stmt(&mut self, s: &TStmt) {
        match s {
            TStmt::FieldSet{base,offset,value} => {
                self.expr(base);
                self.expr(value);
                if value.ty==Type::Bool {self.op(0xAD);} // i64.extend_i32_u
                self.op(0x37); // i64.store
                uleb(&mut self.code,3); // alignment 8
                uleb(&mut self.code,*offset as u64);
            }
            TStmt::Set(slot, v) => {
                self.expr(v);
                self.op(0x21);
                uleb(&mut self.code, *slot as u64);
            }
            TStmt::Expr(v) => {
                self.expr(v);
                if v.ty != Type::Void {
                    self.op(0x1A);
                }
            }
            TStmt::Return(v) => {
                if let Some(v) = v {
                    self.expr(v);
                } else if self.void_main {
                    self.i64c(0);
                }
                self.op(0x0F);
            }
            TStmt::If(arms, other) => self.if_chain(arms, other.as_deref()),
            TStmt::Loop { cond, body, update } => {
                self.open(0x02, 0x40, Frame::Break);
                self.open(0x03, 0x40, Frame::Top);
                if let Some(c) = cond {
                    self.expr(c);
                    self.op(0x45); // i32.eqz
                    self.op(0x0D); // br_if
                    uleb(&mut self.code, 1);
                }
                self.open(0x02, 0x40, Frame::Continue);
                self.stmts(body);
                self.close();
                self.stmts(update);
                self.br_to(Frame::Top);
                self.close();
                self.close();
            }
            TStmt::Break => self.br_to(Frame::Break),
            TStmt::Continue => self.br_to(Frame::Continue),
            TStmt::Block(b) => self.stmts(b),
        }
    }

    fn if_chain(&mut self, arms: &[(TE, Vec<TStmt>)], other: Option<&[TStmt]>) {
        let (c, body) = &arms[0];
        self.expr(c);
        self.open(0x04, 0x40, Frame::Other);
        self.stmts(body);
        if arms.len() > 1 {
            self.op(0x05);
            self.if_chain(&arms[1..], other);
        } else if let Some(o) = other {
            self.op(0x05);
            self.stmts(o);
        }
        self.close();
    }

    fn expr(&mut self, v: &TE) {
        match &v.e {
            TExpr::I64(n) => self.i64c(*n),
            TExpr::Bool(b) => self.i32c(*b as i32),
            TExpr::Local(slot) => {
                self.op(0x20);
                uleb(&mut self.code, *slot as u64);
            }
            TExpr::FieldGet{base,offset} => {
                self.expr(base);
                self.op(0x29); // i64.load
                uleb(&mut self.code,3);
                uleb(&mut self.code,*offset as u64);
                if v.ty==Type::Bool {self.op(0xA7);} // i32.wrap_i64
            }
            TExpr::New(obj) => {
                let (size,tag,defaults,constructor)=
                    (&obj.size,&obj.tag,&obj.defaults,&obj.constructor);
                let tmp=self.object_tmp.expect("object allocation reserves temporary local");
                // Fixed 64 KiB per-guest arena, never grows or imports memory.
                // A 0 handle is invalid; each object also carries its class tag.
                self.op(0x23); uleb(&mut self.code,1); // global.get heap cursor
                self.i32c(65536_i32-*size as i32);
                self.op(0x4B); // i32.gt_u
                self.op(0x04); self.op(0x40); // if
                self.i32c(3);
                self.op(0x24); uleb(&mut self.code,0); // panic_code=3
                self.op(0x00); // trap before any out-of-bounds access
                self.op(0x0B);
                self.op(0x23); uleb(&mut self.code,1);
                self.op(0x21); uleb(&mut self.code,tmp as u64);
                self.op(0x23); uleb(&mut self.code,1);
                self.i32c(*size as i32);
                self.op(0x6A); // i32.add
                self.op(0x24); uleb(&mut self.code,1);
                let mut write=|this:&mut Self,offset:u32,value:i64| {
                    this.op(0x20); uleb(&mut this.code,tmp as u64);
                    this.i64c(value);
                    this.op(0x37); uleb(&mut this.code,3);
                    uleb(&mut this.code,offset as u64);
                };
                write(self,0,*tag);
                for (i,default) in defaults.iter().enumerate() {
                    write(self,8+(i as u32)*8,*default);
                }
                if let Some((func,args))=constructor {
                    self.op(0x20); uleb(&mut self.code,tmp as u64);
                    for arg in args {self.expr(arg);}
                    self.call(self.l.func(*func));
                }
                self.op(0x20); uleb(&mut self.code,tmp as u64);
            }
            TExpr::Neg(a) => {
                self.expr(a);
                self.call(self.l.helper(H_NEG));
            }
            TExpr::Not(a) => {
                self.expr(a);
                self.op(0x45);
            }
            TExpr::Bin(op, a, b) => {
                // Canonical AttoLang conditions are eager: both operands
                // execute in source order even when the left decides truth.
                self.expr(a);
                self.expr(b);
                let int = a.ty != Type::Bool;
                match op {
                    BinOp::Add => self.call(self.l.helper(H_ADD)),
                    BinOp::Sub => self.call(self.l.helper(H_SUB)),
                    BinOp::Mul => self.call(self.l.helper(H_MUL)),
                    BinOp::And => self.op(0x71), // i32.and
                    BinOp::Or => self.op(0x72),  // i32.or
                    BinOp::Lt => self.op(0x53),
                    BinOp::Le => self.op(0x57),
                    BinOp::Gt => self.op(0x55),
                    BinOp::Ge => self.op(0x59),
                    BinOp::Eq => self.op(if int { 0x51 } else { 0x46 }),
                    BinOp::Ne => self.op(if int { 0x52 } else { 0x47 }),
                    BinOp::Div => unreachable!("division not admitted by checker"),
                }
            },
            TExpr::Ternary(c, a, b) => {
                self.expr(c);
                self.open(0x04, vt(v.ty).expect("ternary value type"), Frame::Other);
                self.expr(a);
                self.op(0x05);
                self.expr(b);
                self.close();
            }
            TExpr::Call(f, args) => {
                for a in args {
                    self.expr(a);
                }
                self.call(self.l.func(*f));
            }
            TExpr::Builtin(b, x, y) => {
                self.expr(x);
                self.expr(y);
                self.op(match b {
                    Builtin::And => 0x71,
                    Builtin::Or => 0x72,
                    Builtin::Xor => 0x73,
                });
            }
            TExpr::Print { value, newline } => {
                self.expr(value);
                let imp = if value.ty == Type::Int {
                    Import::PrintI64
                } else {
                    Import::PrintBool
                };
                self.call(self.l.import(imp));
                if *newline {
                    self.call(self.l.import(Import::PrintNewline));
                }
            }
            TExpr::PrintStr { offset, len, newline } => {
                self.i32c(*offset as i32);
                self.i32c(*len as i32);
                self.call(self.l.import(Import::PrintStr));
                if *newline {
                    self.call(self.l.import(Import::PrintNewline));
                }
            }
        }
    }
}

/// panic: `i32.const code; global.set $panic_code; unreachable`
fn panic(c: &mut Vec<u8>, code: i32) {
    c.push(0x41);
    sleb(c, code as i64);
    c.extend_from_slice(&[0x24, 0x00, 0x00]);
}

fn helper_bodies() -> Vec<(Sig, Vec<u8>, Vec<u8>)> {
    // (signature, extra locals as (count,type) pairs encoded, code)
    let bin = Sig(vec![I64, I64], vec![I64]);
    let r_local = vec![1u8, 1, I64]; // 1 group: 1 x i64 (local 2)
    let no_locals = vec![0u8];
    let mut v = Vec::new();

    // add: r = a + b; overflow iff ((a ^ r) & (b ^ r)) < 0
    let mut c = vec![
        0x20, 0, 0x20, 1, 0x7C, 0x21, 2, 0x20, 0, 0x20, 2, 0x85, 0x20, 1, 0x20, 2, 0x85, 0x83, 0x42, 0, 0x53, 0x04, 0x40,
    ];
    panic(&mut c, PANIC_OVERFLOW);
    c.extend_from_slice(&[0x0B, 0x20, 2, 0x0B]);
    v.push((bin.clone(), r_local.clone(), c));

    // sub: r = a - b; overflow iff ((a ^ b) & (a ^ r)) < 0
    let mut c = vec![
        0x20, 0, 0x20, 1, 0x7D, 0x21, 2, 0x20, 0, 0x20, 1, 0x85, 0x20, 0, 0x20, 2, 0x85, 0x83, 0x42, 0, 0x53, 0x04, 0x40,
    ];
    panic(&mut c, PANIC_OVERFLOW);
    c.extend_from_slice(&[0x0B, 0x20, 2, 0x0B]);
    v.push((bin.clone(), r_local.clone(), c));

    // mul
    let mut c = vec![0x20, 0, 0x42, 0x7F, 0x51, 0x04, 0x40]; // if a == -1
    c.extend_from_slice(&[0x20, 1]);
    c.push(0x42);
    sleb(&mut c, i64::MIN);
    c.extend_from_slice(&[0x51, 0x04, 0x40]); // if b == MIN
    panic(&mut c, PANIC_OVERFLOW);
    c.extend_from_slice(&[0x0B, 0x42, 0, 0x20, 1, 0x7D, 0x0F, 0x0B]); // return 0 - b
    c.extend_from_slice(&[0x20, 0, 0x50, 0x04, 0x40, 0x42, 0, 0x0F, 0x0B]); // if a == 0 return 0
    c.extend_from_slice(&[0x20, 0, 0x20, 1, 0x7E, 0x21, 2]); // r = a * b
    c.extend_from_slice(&[0x20, 2, 0x20, 0, 0x7F, 0x20, 1, 0x52, 0x04, 0x40]); // if r / a != b
    panic(&mut c, PANIC_OVERFLOW);
    c.extend_from_slice(&[0x0B, 0x20, 2, 0x0B]);
    v.push((bin.clone(), r_local, c));

    // neg
    let mut c = vec![0x20, 0, 0x42];
    sleb(&mut c, i64::MIN);
    c.extend_from_slice(&[0x51, 0x04, 0x40]);
    panic(&mut c, PANIC_OVERFLOW);
    c.extend_from_slice(&[0x0B, 0x42, 0, 0x20, 0, 0x7D, 0x0B]);
    v.push((Sig(vec![I64], vec![I64]), no_locals.clone(), c));

    v
}

fn locals_decl(types: &[Type]) -> Vec<u8> {
    let mut groups: Vec<(u32, u8)> = Vec::new();
    for t in types {
        let b = vt(*t).expect("value local");
        match groups.last_mut() {
            Some((n, ty)) if *ty == b => *n += 1,
            _ => groups.push((1, b)),
        }
    }
    let mut out = Vec::new();
    uleb(&mut out, groups.len() as u64);
    for (n, t) in groups {
        uleb(&mut out, n as u64);
        out.push(t);
    }
    out
}

pub fn emit(p: &TProgram) -> Vec<u8> {
    let layout = Layout {
        imports: p.imports.clone(),
    };
    let mut types = Types(Vec::new());

    let mut import_sec = Vec::new();
    uleb(&mut import_sec, p.imports.len() as u64);
    for imp in &p.imports {
        let sig = match imp {
            Import::PrintI64 => Sig(vec![I64], vec![]),
            Import::PrintBool => Sig(vec![I32], vec![]),
            Import::PrintStr => Sig(vec![I32, I32], vec![]),
            Import::PrintNewline => Sig(vec![], vec![]),
        };
        name(&mut import_sec, "atto");
        name(&mut import_sec, imp.name());
        import_sec.push(0x00);
        uleb(&mut import_sec, types.idx(sig) as u64);
    }

    let mut func_types = Vec::new();
    let mut bodies = Vec::new();
    for (sig, locals, code) in helper_bodies() {
        func_types.push(types.idx(sig));
        let mut b = locals;
        b.extend(code);
        bodies.push(b);
    }
    let main_void = p.funcs[p.main as usize].ret == Type::Void;
    for (i, f) in p.funcs.iter().enumerate() {
        let params: Vec<u8> = f.params.iter().map(|t| vt(*t).unwrap()).collect();
        let is_void_main = i as u32 == p.main && main_void;
        let results: Vec<u8> = if is_void_main { vec![I64] } else { vt(f.ret).into_iter().collect() };
        func_types.push(types.idx(Sig(params, results)));
        let mut locals=f.locals.clone();
        if p.uses_objects {locals.push(Type::Class(0));}
        let mut e = Emit {
            l: &layout,
            code: locals_decl(&locals),
            frames: Vec::new(),
            void_main: is_void_main,
            object_tmp: if p.uses_objects {
                Some((f.params.len()+f.locals.len()) as u32)
            } else {None},
        };
        e.stmts(&f.body);
        if is_void_main {
            e.i64c(0);
        } else if f.ret != Type::Void {
            e.op(0x00); // unreachable: definite-return analysis proved every path returns
        }
        e.op(0x0B);
        bodies.push(e.code);
    }

    let mut out = vec![0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00];

    let mut type_sec = Vec::new();
    uleb(&mut type_sec, types.0.len() as u64);
    for Sig(ps, rs) in &types.0 {
        type_sec.push(0x60);
        uleb(&mut type_sec, ps.len() as u64);
        type_sec.extend(ps);
        uleb(&mut type_sec, rs.len() as u64);
        type_sec.extend(rs);
    }
    section(&mut out, 1, type_sec);
    if !p.imports.is_empty() {
        section(&mut out, 2, import_sec);
    }

    let mut fsec = Vec::new();
    uleb(&mut fsec, func_types.len() as u64);
    for t in &func_types {
        uleb(&mut fsec, *t as u64);
    }
    section(&mut out, 3, fsec);

    let has_memory = !p.data.is_empty() || p.uses_objects;
    if has_memory {
        // one fixed, non-growable page holding the string literals
        section(&mut out, 5, vec![1, 0x01, 1, 1]);
    }

    // global 0: mut i32 panic_code = 0; optional global 1: heap cursor.
    let mut globals=vec![if p.uses_objects {2} else {1}, I32, 0x01, 0x41, 0, 0x0B];
    if p.uses_objects {
        // Strings occupy the low segment; skip the reserved zero handle and
        // align heap objects to 8 bytes for i64 slots.
        let heap_base=(p.data.len().max(8)+7)&!7;
        globals.extend([I32,0x01,0x41]);
        sleb(&mut globals,heap_base as i64);
        globals.push(0x0B);
    }
    section(&mut out,6,globals);

    let mut exp = Vec::new();
    uleb(&mut exp, if has_memory { 3 } else { 2 });
    name(&mut exp, "main");
    exp.push(0x00);
    uleb(&mut exp, layout.func(p.main) as u64);
    name(&mut exp, "panic_code");
    exp.push(0x03);
    uleb(&mut exp, 0);
    if has_memory {
        name(&mut exp, "memory");
        exp.push(0x02);
        uleb(&mut exp, 0);
    }
    section(&mut out, 7, exp);

    let mut code_sec = Vec::new();
    uleb(&mut code_sec, bodies.len() as u64);
    for b in bodies {
        uleb(&mut code_sec, b.len() as u64);
        code_sec.extend(b);
    }
    section(&mut out, 10, code_sec);

    if !p.data.is_empty() {
        let mut d = vec![1, 0x00, 0x41, 0x00, 0x0B];
        uleb(&mut d, p.data.len() as u64);
        d.extend_from_slice(&p.data);
        section(&mut out, 11, d);
    }

    // custom "name" section: function names for readable traps and stack traces
    let mut names = Vec::new();
    let all: Vec<String> = p
        .imports
        .iter()
        .map(|i| format!("atto.{}", i.name()))
        .chain(
            [
                "atto.checked_add",
                "atto.checked_sub",
                "atto.checked_mul",
                "atto.checked_neg",
            ]
            .map(String::from),
        )
        .chain(p.funcs.iter().map(|f| f.name.clone()))
        .collect();
    let mut sub = Vec::new();
    uleb(&mut sub, all.len() as u64);
    for (i, n) in all.iter().enumerate() {
        uleb(&mut sub, i as u64);
        name(&mut sub, n);
    }
    name(&mut names, "name");
    names.push(1);
    uleb(&mut names, sub.len() as u64);
    names.extend(sub);
    section(&mut out, 0, names);
    out
}
