//! v2 IR -> WebAssembly (same guest contract `atto.browser.guest.v2`:
//! exports `main`, `panic_code`, `memory`; imports only `atto.print_*`).
//!
//! A suspending function is emitted as a state machine. Its body is split
//! into basic blocks wherever control flow contains a suspension point, and
//! a `br_table` dispatch loop selects the block from a `pc` local. When a
//! callee starts unwinding (global state 1), the function saves every local
//! and its `pc` on the current task's frame stack and returns; when the
//! scheduler rewinds the task (state 2) the function restores them and
//! re-enters the block that made the pending call. No host stack switching
//! or extra imports are involved.

use super::ir::*;
use crate::wasm::{helper_bodies, name, section, sleb, uleb, Sig, HEAP_PAGES};

const I32: u8 = 0x7F;
const I64: u8 = 0x7E;

/// panic_code: saved frames exceed the task's frame stack
pub const PANIC_TASK_STACK: i32 = 11;

const HELPERS: u32 = 5; // checked add, sub, mul, neg, alloc

struct Layout {
    imports: [Option<u32>; 4],
    first_helper: u32,
    first_fn: u32,
    /// type index of `(i64 x n) -> i64`
    arity_types: Vec<u32>,
}

impl Layout {
    fn func(&self, id: u32) -> u32 {
        self.first_fn + id
    }
    fn helper(&self, h: Helper) -> u32 {
        self.first_helper
            + match h {
                Helper::Add => 0,
                Helper::Sub => 1,
                Helper::Mul => 2,
                Helper::Neg => 3,
                Helper::Alloc => 4,
            }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Label {
    Exit,
    Cont,
    Other,
    Dispatch,
    Save,
}

struct Emit<'a> {
    l: &'a Layout,
    code: Vec<u8>,
    labels: Vec<Label>,
}

impl Emit<'_> {
    fn op(&mut self, b: u8) {
        self.code.push(b);
    }
    fn i64c(&mut self, v: i64) {
        self.code.push(0x42);
        sleb(&mut self.code, v);
    }
    fn i32c(&mut self, v: i32) {
        self.code.push(0x41);
        sleb(&mut self.code, v as i64);
    }
    fn idx(&mut self, opcode: u8, i: u32) {
        self.code.push(opcode);
        uleb(&mut self.code, i as u64);
    }
    fn open(&mut self, opcode: u8, blocktype: u8, label: Label) {
        self.code.extend_from_slice(&[opcode, blocktype]);
        self.labels.push(label);
    }
    fn close(&mut self) {
        self.code.push(0x0B);
        self.labels.pop();
    }
    fn br(&mut self, opcode: u8, to: Label) {
        let depth = self.labels.iter().rev().position(|l| *l == to).expect("branch target is open");
        self.idx(opcode, depth as u32);
    }
    fn mem(&mut self, opcode: u8, align: u8, offset: u32) {
        self.code.extend_from_slice(&[opcode, align]);
        uleb(&mut self.code, offset as u64);
    }
    fn panic(&mut self, code: i32) {
        self.i32c(code);
        self.idx(0x24, G_PANIC);
        self.op(0x00);
    }

    /// Leaves an i32 truth value on the stack.
    fn cond(&mut self, e: &Ir) {
        match e {
            Ir::Bin(op, a, b) if cmp_opcode(*op).is_some() => {
                self.expr(a);
                self.expr(b);
                self.op(cmp_opcode(*op).unwrap());
            }
            Ir::Not(x) => {
                self.expr(x);
                self.op(0x50);
            }
            _ => {
                self.expr(e);
                self.i64c(0);
                self.op(0x52);
            }
        }
    }

    fn expr(&mut self, e: &Ir) {
        match e {
            Ir::Const(v) => self.i64c(*v),
            Ir::Local(s) => self.idx(0x20, *s),
            Ir::Global(g) => {
                self.idx(0x23, *g);
                self.op(0xAD);
            }
            Ir::Load(a) => {
                self.expr(a);
                self.op(0xA7);
                self.mem(0x29, 3, 0);
            }
            Ir::Load8(a) => {
                self.expr(a);
                self.op(0xA7);
                self.mem(0x31, 0, 0);
            }
            Ir::Bin(op, a, b) => {
                self.expr(a);
                self.expr(b);
                match op {
                    Op::AddRaw => self.op(0x7C),
                    Op::SubRaw => self.op(0x7D),
                    Op::MulRaw => self.op(0x7E),
                    Op::And => self.op(0x83),
                    Op::Or => self.op(0x84),
                    Op::Xor => self.op(0x85),
                    Op::Shl => self.op(0x86),
                    Op::ShrU => self.op(0x88),
                    cmp => {
                        self.op(cmp_opcode(*cmp).unwrap());
                        self.op(0xAD);
                    }
                }
            }
            Ir::Not(x) => {
                self.expr(x);
                self.op(0x50);
                self.op(0xAD);
            }
            Ir::Call(id, args) => {
                for a in args {
                    self.expr(a);
                }
                let f = self.l.func(*id);
                self.idx(0x10, f);
            }
            Ir::CallClosure(clo, args) => {
                debug_assert!(matches!(**clo, Ir::Local(_) | Ir::Const(_)));
                self.expr(clo);
                for a in args {
                    self.expr(a);
                }
                self.expr(clo);
                self.op(0xA7);
                self.mem(0x29, 3, 0);
                self.op(0xA7);
                let ty = self.l.arity_types[args.len() + 1];
                self.idx(0x11, ty);
                self.op(0x00);
            }
            Ir::Helper(h, args) => {
                for a in args {
                    self.expr(a);
                }
                let f = self.l.helper(*h);
                self.idx(0x10, f);
            }
            Ir::If(c, a, b) => {
                self.cond(c);
                self.open(0x04, I64, Label::Other);
                self.expr(a);
                self.op(0x05);
                self.expr(b);
                self.close();
            }
        }
    }

    fn stmts(&mut self, s: &[IrStmt]) {
        for x in s {
            self.stmt(x);
        }
    }

    fn stmt(&mut self, s: &IrStmt) {
        match s {
            IrStmt::Set(slot, v) => {
                self.expr(v);
                self.idx(0x21, *slot);
            }
            IrStmt::Store(addr, v) => {
                self.expr(addr);
                self.op(0xA7);
                self.expr(v);
                self.mem(0x37, 3, 0);
            }
            IrStmt::Store8(addr, v) => {
                self.expr(addr);
                self.op(0xA7);
                self.expr(v);
                self.mem(0x3C, 0, 0);
            }
            IrStmt::GlobalSet(g, v) => {
                self.expr(v);
                self.op(0xA7);
                self.idx(0x24, *g);
            }
            IrStmt::Expr(v) => {
                self.expr(v);
                self.op(0x1A);
            }
            IrStmt::If(c, a, b) => {
                self.cond(c);
                self.open(0x04, 0x40, Label::Other);
                self.stmts(a);
                if !b.is_empty() {
                    self.op(0x05);
                    self.stmts(b);
                }
                self.close();
            }
            IrStmt::Loop { body, update } => {
                self.open(0x02, 0x40, Label::Exit);
                self.open(0x03, 0x40, Label::Other);
                self.open(0x02, 0x40, Label::Cont);
                self.stmts(body);
                self.close();
                self.stmts(update);
                self.idx(0x0C, 0);
                self.close();
                self.close();
            }
            IrStmt::Break => self.br(0x0C, Label::Exit),
            IrStmt::Continue => self.br(0x0C, Label::Cont),
            IrStmt::Return(v) => {
                self.expr(v);
                self.op(0x0F);
            }
            IrStmt::Print(kind, v) => {
                let import = self.l.imports[*kind as usize].expect("print import is declared");
                match kind {
                    Print::I64 => self.expr(v.as_ref().unwrap()),
                    Print::Bool => {
                        self.expr(v.as_ref().unwrap());
                        self.op(0xA7);
                    }
                    Print::Str => {
                        let v = v.as_ref().unwrap();
                        debug_assert!(matches!(v, Ir::Local(_) | Ir::Const(_)));
                        self.expr(v);
                        self.i64c(32);
                        self.op(0x88);
                        self.op(0xA7);
                        self.expr(v);
                        self.op(0xA7);
                    }
                    Print::Newline => {}
                }
                self.idx(0x10, import);
            }
            IrStmt::Panic(code) => self.panic(*code),
            IrStmt::SCall(..) | IrStmt::Suspend => {
                unreachable!("suspension points are lowered by the state-machine emitter")
            }
        }
    }
}

fn cmp_opcode(op: Op) -> Option<u8> {
    Some(match op {
        Op::Eq => 0x51,
        Op::Ne => 0x52,
        Op::LtS => 0x53,
        Op::GtS => 0x55,
        Op::LeS => 0x57,
        Op::GeS => 0x59,
        _ => return None,
    })
}

// ------------------------------------------------------------ state machine

enum Term {
    /// not yet terminated; an unreachable block if left like this
    Open,
    Goto(usize),
    Cond(Ir, usize, usize),
    SCall { dst: Option<u32>, f: u32, args: Vec<Ir>, next: usize },
    Suspend { next: usize },
}

struct Block {
    stmts: Vec<IrStmt>,
    term: Term,
}

struct Cfg {
    blocks: Vec<Block>,
    cur: usize,
}

fn has_suspension(s: &IrStmt) -> bool {
    match s {
        IrStmt::SCall(..) | IrStmt::Suspend => true,
        IrStmt::If(_, a, b) => a.iter().any(has_suspension) || b.iter().any(has_suspension),
        IrStmt::Loop { body, update } => body.iter().any(has_suspension) || update.iter().any(has_suspension),
        _ => false,
    }
}

/// A `break`/`continue` that leaves `s` (not enclosed by a loop inside it).
fn has_escape(s: &IrStmt) -> bool {
    match s {
        IrStmt::Break | IrStmt::Continue => true,
        IrStmt::If(_, a, b) => a.iter().any(has_escape) || b.iter().any(has_escape),
        _ => false,
    }
}

impl Cfg {
    fn new_block(&mut self) -> usize {
        self.blocks.push(Block { stmts: Vec::new(), term: Term::Open });
        self.blocks.len() - 1
    }
    fn goto(&mut self, to: usize) {
        if matches!(self.blocks[self.cur].term, Term::Open) {
            self.blocks[self.cur].term = Term::Goto(to);
        }
    }
    /// `loops` holds `(continue target, break target)` of flattened loops.
    fn flat(&mut self, stmts: &[IrStmt], loops: &mut Vec<(usize, usize)>) {
        for s in stmts {
            if !(has_suspension(s) || (!loops.is_empty() && has_escape(s))) {
                self.blocks[self.cur].stmts.push(s.clone());
                continue;
            }
            match s {
                IrStmt::SCall(dst, f, args) => {
                    // The call owns a block: rewinding re-enters exactly it.
                    let call = self.new_block();
                    let next = self.new_block();
                    self.goto(call);
                    self.blocks[call].term = Term::SCall { dst: *dst, f: *f, args: args.clone(), next };
                    self.cur = next;
                }
                IrStmt::Suspend => {
                    let point = self.new_block();
                    let next = self.new_block();
                    self.goto(point);
                    self.blocks[point].term = Term::Suspend { next };
                    self.cur = next;
                }
                IrStmt::If(c, a, b) => {
                    let (ta, tb, join) = (self.new_block(), self.new_block(), self.new_block());
                    self.blocks[self.cur].term = Term::Cond(c.clone(), ta, tb);
                    self.cur = ta;
                    self.flat(a, loops);
                    self.goto(join);
                    self.cur = tb;
                    self.flat(b, loops);
                    self.goto(join);
                    self.cur = join;
                }
                IrStmt::Loop { body, update } => {
                    let (head, cont, exit) = (self.new_block(), self.new_block(), self.new_block());
                    self.goto(head);
                    self.cur = head;
                    loops.push((cont, exit));
                    self.flat(body, loops);
                    self.goto(cont);
                    loops.pop();
                    self.cur = cont;
                    self.flat(update, loops);
                    self.goto(head);
                    self.cur = exit;
                }
                IrStmt::Break => {
                    let target = loops.last().expect("break inside a loop").1;
                    self.goto(target);
                    self.cur = self.new_block();
                }
                IrStmt::Continue => {
                    let target = loops.last().expect("continue inside a loop").0;
                    self.goto(target);
                    self.cur = self.new_block();
                }
                _ => unreachable!("only control flow needs flattening"),
            }
        }
    }
}

fn state_machine(l: &Layout, f: &IrFn) -> Vec<u8> {
    let mut cfg = Cfg { blocks: Vec::new(), cur: 0 };
    cfg.new_block();
    cfg.flat(&f.body, &mut Vec::new());
    let n = cfg.blocks.len();
    let pc = f.locals;
    let frame = 8 * (f.locals + 1);

    // locals: i64 x (locals - params), then pc: i32
    let mut code = Vec::new();
    uleb(&mut code, 2);
    uleb(&mut code, (f.locals - f.params) as u64);
    code.push(I64);
    uleb(&mut code, 1);
    code.push(I32);
    let mut e = Emit { l, code, labels: Vec::new() };

    // rewinding: pop this function's frame
    e.idx(0x23, G_STATE);
    e.i32c(STATE_REWIND);
    e.op(0x46);
    e.open(0x04, 0x40, Label::Other);
    e.idx(0x23, G_FSP);
    e.i32c(frame as i32);
    e.op(0x6B);
    e.idx(0x24, G_FSP);
    for i in 0..f.locals {
        e.idx(0x23, G_FSP);
        e.mem(0x29, 3, 8 * i);
        e.idx(0x21, i);
    }
    e.idx(0x23, G_FSP);
    e.mem(0x29, 3, 8 * f.locals);
    e.op(0xA7);
    e.idx(0x21, pc);
    e.close();

    e.open(0x02, 0x40, Label::Save);
    e.open(0x03, 0x40, Label::Dispatch);
    for _ in 0..n {
        e.open(0x02, 0x40, Label::Other);
    }
    e.idx(0x20, pc);
    e.op(0x0E);
    uleb(&mut e.code, n as u64);
    for i in 0..=n {
        // the innermost open block is block 0; an out-of-range pc lands on it
        uleb(&mut e.code, if i < n { i as u64 } else { 0 });
    }
    let jump = |e: &mut Emit, to: usize| {
        e.i32c(to as i32);
        e.idx(0x21, pc);
        e.br(0x0C, Label::Dispatch);
    };
    for b in &cfg.blocks {
        e.close();
        e.stmts(&b.stmts);
        match &b.term {
            // falling off the end of the body (or dead code after a jump)
            Term::Open => {
                e.i64c(0);
                e.op(0x0F);
            }
            Term::Goto(to) => jump(&mut e, *to),
            Term::Cond(c, a, bb) => {
                e.cond(c);
                e.open(0x04, 0x40, Label::Other);
                jump(&mut e, *a);
                e.close();
                jump(&mut e, *bb);
            }
            Term::SCall { dst, f: callee, args, next } => {
                for a in args {
                    debug_assert!(matches!(a, Ir::Local(_) | Ir::Const(_)));
                    e.expr(a);
                }
                let idx = l.func(*callee);
                e.idx(0x10, idx);
                e.idx(0x23, G_STATE);
                e.i32c(STATE_UNWIND);
                e.op(0x46);
                e.br(0x0D, Label::Save);
                match dst {
                    Some(d) => e.idx(0x21, *d),
                    None => e.op(0x1A),
                }
                jump(&mut e, *next);
            }
            Term::Suspend { next } => {
                e.idx(0x23, G_STATE);
                e.i32c(STATE_REWIND);
                e.op(0x46);
                e.open(0x04, 0x40, Label::Other);
                e.i32c(STATE_RUN);
                e.idx(0x24, G_STATE);
                jump(&mut e, *next);
                e.close();
                e.i32c(STATE_UNWIND);
                e.idx(0x24, G_STATE);
                e.br(0x0C, Label::Save);
            }
        }
    }
    e.close(); // dispatch loop
    e.op(0x00);
    e.close(); // save

    // unwinding: push this function's frame and return to the caller
    e.idx(0x23, G_FSP);
    e.i32c(frame as i32);
    e.op(0x6A);
    e.idx(0x23, G_FEND);
    e.op(0x4B);
    e.open(0x04, 0x40, Label::Other);
    e.panic(PANIC_TASK_STACK);
    e.close();
    for i in 0..f.locals {
        e.idx(0x23, G_FSP);
        e.idx(0x20, i);
        e.mem(0x37, 3, 8 * i);
    }
    e.idx(0x23, G_FSP);
    e.idx(0x20, pc);
    e.op(0xAD);
    e.mem(0x37, 3, 8 * f.locals);
    e.idx(0x23, G_FSP);
    e.i32c(frame as i32);
    e.op(0x6A);
    e.idx(0x24, G_FSP);
    e.i64c(0);
    e.op(0x0B);
    e.code
}

fn plain(l: &Layout, f: &IrFn) -> Vec<u8> {
    let mut code = Vec::new();
    if f.locals > f.params {
        uleb(&mut code, 1);
        uleb(&mut code, (f.locals - f.params) as u64);
        code.push(I64);
    } else {
        uleb(&mut code, 0);
    }
    let mut e = Emit { l, code, labels: Vec::new() };
    e.stmts(&f.body);
    e.i64c(0);
    e.op(0x0B);
    e.code
}

/// `alloc(size: i64) -> i64`: bump-allocate zeroed, 8-byte aligned memory;
/// panic 3 when the fixed 1 MiB memory is exhausted. Nothing is freed.
fn alloc_body() -> Vec<u8> {
    let limit = (HEAP_PAGES * 65536) as i64;
    let mut c = vec![1, 2, I32]; // locals: p (1), end (2)
    c.extend_from_slice(&[0x23, G_HEAP as u8, 0x21, 1]);
    c.extend_from_slice(&[0x20, 1, 0x20, 0, 0xA7, 0x41, 7, 0x6A, 0x41, 0x78, 0x71, 0x6A, 0x21, 2]);
    c.extend_from_slice(&[0x20, 0, 0x42]);
    sleb(&mut c, limit);
    c.push(0x56); // size >u limit
    c.extend_from_slice(&[0x20, 2, 0x41]);
    sleb(&mut c, limit);
    c.extend_from_slice(&[0x4B, 0x72]); // end >u limit
    c.extend_from_slice(&[0x20, 2, 0x20, 1, 0x49, 0x72]); // end <u p (wrapped)
    c.extend_from_slice(&[0x04, 0x40, 0x41, 3, 0x24, G_PANIC as u8, 0x00, 0x0B]);
    c.extend_from_slice(&[0x20, 2, 0x24, G_HEAP as u8, 0x20, 1, 0xAD, 0x0B]);
    c
}

pub fn emit(m: &Module) -> Vec<u8> {
    let mut types: Vec<Sig> = Vec::new();
    let type_idx = |types: &mut Vec<Sig>, s: Sig| -> u32 {
        if let Some(i) = types.iter().position(|x| *x == s) {
            return i as u32;
        }
        types.push(s);
        (types.len() - 1) as u32
    };

    let import_sigs = [
        ("print_i64", Sig(vec![I64], vec![])),
        ("print_bool", Sig(vec![I32], vec![])),
        ("print_str", Sig(vec![I32, I32], vec![])),
        ("print_newline", Sig(vec![], vec![])),
    ];
    let mut imports = [None; 4];
    let mut import_sec = Vec::new();
    let mut n_imports = 0u32;
    for (i, (nm, sig)) in import_sigs.iter().enumerate() {
        if !m.uses_print[i] {
            continue;
        }
        imports[i] = Some(n_imports);
        n_imports += 1;
        name(&mut import_sec, "atto");
        name(&mut import_sec, nm);
        import_sec.push(0x00);
        uleb(&mut import_sec, type_idx(&mut types, Sig(sig.0.clone(), sig.1.clone())) as u64);
    }
    let mut counted = Vec::new();
    uleb(&mut counted, n_imports as u64);
    counted.extend(import_sec);

    let max_arity = m.fns.iter().map(|f| f.params as usize).max().unwrap_or(0).max(8) + 1;
    let arity_types: Vec<u32> =
        (0..=max_arity).map(|k| type_idx(&mut types, Sig(vec![I64; k], vec![I64]))).collect();
    let layout = Layout { imports, first_helper: n_imports, first_fn: n_imports + HELPERS, arity_types };

    let mut func_types = Vec::new();
    let mut bodies = Vec::new();
    for (sig, locals, code) in helper_bodies() {
        func_types.push(type_idx(&mut types, sig));
        let mut b = locals;
        b.extend(code);
        bodies.push(b);
    }
    func_types.push(layout.arity_types[1]);
    bodies.push(alloc_body());
    for f in &m.fns {
        func_types.push(layout.arity_types[f.params as usize]);
        bodies.push(if f.suspends { state_machine(&layout, f) } else { plain(&layout, f) });
    }

    let mut out = vec![0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00];
    let mut type_sec = Vec::new();
    uleb(&mut type_sec, types.len() as u64);
    for Sig(ps, rs) in &types {
        type_sec.push(0x60);
        uleb(&mut type_sec, ps.len() as u64);
        type_sec.extend(ps);
        uleb(&mut type_sec, rs.len() as u64);
        type_sec.extend(rs);
    }
    section(&mut out, 1, type_sec);
    if n_imports > 0 {
        section(&mut out, 2, counted);
    }
    let mut fsec = Vec::new();
    uleb(&mut fsec, func_types.len() as u64);
    for t in &func_types {
        uleb(&mut fsec, *t as u64);
    }
    section(&mut out, 3, fsec);
    if !m.table.is_empty() {
        let mut t = vec![1, 0x70, 0x01];
        uleb(&mut t, m.table.len() as u64);
        uleb(&mut t, m.table.len() as u64);
        section(&mut out, 4, t);
    }
    let mut mem = vec![1, 0x01];
    uleb(&mut mem, HEAP_PAGES as u64);
    uleb(&mut mem, HEAP_PAGES as u64);
    section(&mut out, 5, mem);

    let heap_start = ((DATA_BASE as i64 + m.data.len() as i64) + 7) & !7;
    let mut gsec = vec![5];
    gsec.extend_from_slice(&[I32, 0x01, 0x41, 0, 0x0B]);
    gsec.extend_from_slice(&[I32, 0x01, 0x41]);
    sleb(&mut gsec, heap_start);
    gsec.push(0x0B);
    for _ in 0..3 {
        gsec.extend_from_slice(&[I32, 0x01, 0x41, 0, 0x0B]);
    }
    section(&mut out, 6, gsec);

    let mut exp = vec![3];
    name(&mut exp, "main");
    exp.push(0x00);
    uleb(&mut exp, layout.func(m.main) as u64);
    name(&mut exp, "panic_code");
    exp.push(0x03);
    uleb(&mut exp, G_PANIC as u64);
    name(&mut exp, "memory");
    exp.push(0x02);
    uleb(&mut exp, 0);
    section(&mut out, 7, exp);

    if !m.table.is_empty() {
        let mut el = vec![1, 0x00, 0x41, 0x00, 0x0B];
        uleb(&mut el, m.table.len() as u64);
        for id in &m.table {
            uleb(&mut el, layout.func(*id) as u64);
        }
        section(&mut out, 9, el);
    }

    let mut code_sec = Vec::new();
    uleb(&mut code_sec, bodies.len() as u64);
    for b in bodies {
        uleb(&mut code_sec, b.len() as u64);
        code_sec.extend(b);
    }
    section(&mut out, 10, code_sec);

    if !m.data.is_empty() {
        let mut d = vec![1, 0x00, 0x41];
        sleb(&mut d, DATA_BASE as i64);
        d.push(0x0B);
        uleb(&mut d, m.data.len() as u64);
        d.extend_from_slice(&m.data);
        section(&mut out, 11, d);
    }
    out
}
