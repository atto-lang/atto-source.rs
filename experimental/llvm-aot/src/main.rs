//! AOT lowering for a closed, scalar Atto subset: checked typed frontend -> LLVM IR.
//! Unsupported guest constructs are explicit errors, never silently stubbed.
use std::fmt::Write as _;
use std::io::{Read as IoRead, Write as IoWrite};
use std::process::{Command, Stdio};

mod target_native;

use atto_compiler::ast::{BinOp, Type};
use atto_compiler::check::{self, Import, TExpr, TFunc, TProgram, TStmt, TE};

fn scalar_type(t: Type) -> Result<&'static str, String> {
    match t {
        Type::Int => Ok("i64"),
        Type::Bool => Ok("i1"),
        Type::Void => Ok("void"),
        // A generation checked capability, never an address or raw state pointer.
        Type::ActorRef(_) => Ok("i32"),
        _ => Err(format!("E_UNSUPPORTED_NATIVE_TYPE: {}", t.name())),
    }
}

struct FunctionEmitter<'a> {
    program: &'a TProgram,
    func: &'a TFunc,
    out: String,
    counter: u32,
    terminated: bool,
    block_name: String,
    slots: Vec<Type>,
}

impl<'a> FunctionEmitter<'a> {
    fn new(program: &'a TProgram, func: &'a TFunc) -> Result<Self, String> {
        let mut slots = func.params.clone();
        slots.extend(func.locals.iter().copied());
        for t in &slots {
            if scalar_type(*t)? == "void" {
                return Err("E_UNSUPPORTED_NATIVE_TYPE: void local".into());
            }
        }
        scalar_type(func.ret)?;
        Ok(Self {
            program, func, out: String::new(), counter: 0,
            terminated: false, block_name: String::new(), slots,
        })
    }

    fn id(&mut self, prefix: &str) -> String {
        self.counter += 1;
        format!("{prefix}{}", self.counter)
    }

    fn instr(&mut self, instruction: impl AsRef<str>) {
        let _ = writeln!(self.out, "  {}", instruction.as_ref());
    }

    fn label(&mut self, name: &str) {
        let _ = writeln!(self.out, "{name}:");
        self.block_name = name.to_string();
        self.terminated = false;
    }

    fn branch(&mut self, name: &str) {
        self.instr(format!("br label %{name}"));
        self.terminated = true;
    }

    fn checked_math(&mut self, opcode: &str, left: &str, right: &str) -> String {
        let pair = self.id("v");
        let overflow = self.id("v");
        let trap = self.id("trap");
        let okay = self.id("okay");
        let result = self.id("v");
        self.instr(format!("%{pair} = call {{ i64, i1 }} @llvm.{opcode}.with.overflow.i64(i64 {left}, i64 {right})"));
        self.instr(format!("%{overflow} = extractvalue {{ i64, i1 }} %{pair}, 1"));
        self.instr(format!("br i1 %{overflow}, label %{trap}, label %{okay}"));
        self.terminated = true;
        self.label(&trap);
        self.instr("call void @llvm.trap()");
        self.instr("unreachable");
        self.terminated = true;
        self.label(&okay);
        self.instr(format!("%{result} = extractvalue {{ i64, i1 }} %{pair}, 0"));
        format!("%{result}")
    }

    fn checked_actor_result(&mut self, result: &str, ty: &str, bad: &str) {
        let is_bad = self.id("actor_fail");
        let rejected = self.id("actor_rejected");
        let admitted = self.id("actor_admitted");
        self.instr(format!("%{is_bad} = icmp eq {ty} {result}, {bad}"));
        self.instr(format!("br i1 %{is_bad}, label %{rejected}, label %{admitted}"));
        self.terminated = true;
        self.label(&rejected);
        self.instr("call void @llvm.trap()");
        self.instr("unreachable");
        self.terminated = true;
        self.label(&admitted);
    }

    fn expr(&mut self, expr: &TE) -> Result<String, String> {
        match &expr.e {
            TExpr::I64(n) => Ok(n.to_string()),
            TExpr::Bool(b) => Ok(if *b { "true" } else { "false" }.to_string()),
            TExpr::Local(slot) => {
                let idx = *slot as usize;
                let ty = scalar_type(*self.slots.get(idx).ok_or("E_INTERNAL: bad local slot")?)?;
                let tmp = self.id("v");
                self.instr(format!("%{tmp} = load {ty}, ptr %local{idx}"));
                Ok(format!("%{tmp}"))
            }
            TExpr::Neg(value) => {
                if value.ty != Type::Int {
                    return Err("E_UNSUPPORTED_NATIVE_NEG: expected int".into());
                }
                let x = self.expr(value)?;
                Ok(self.checked_math("ssub", "0", &x))
            }
            TExpr::Not(value) => {
                if value.ty != Type::Bool {
                    return Err("E_UNSUPPORTED_NATIVE_NOT: expected bool".into());
                }
                let x = self.expr(value)?;
                let tmp = self.id("v");
                self.instr(format!("%{tmp} = xor i1 {x}, true"));
                Ok(format!("%{tmp}"))
            }
            TExpr::Bin(op, lhs, rhs) => {
                let a = self.expr(lhs)?;
                let b = self.expr(rhs)?;
                let ty = scalar_type(lhs.ty)?;
                match op {
                    BinOp::Add | BinOp::Sub | BinOp::Mul => {
                        if ty != "i64" {
                            return Err("E_UNSUPPORTED_NATIVE_ARITH: expected int".into());
                        }
                        let op_name = match op {
                            BinOp::Add => "sadd",
                            BinOp::Sub => "ssub",
                            _ => "smul",
                        };
                        Ok(self.checked_math(op_name, &a, &b))
                    }
                    BinOp::Div => Err("E_UNSUPPORTED_NATIVE_DIV: integer division is not admitted".into()),
                    // The canonical frontend added typed bitwise/shifts since
                    // the old frozen frontend snapshot. Reject BEFORE any
                    // LLVM native guest artifact is published until their
                    // checked native semantics are implemented and verified.
                    BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor |
                    BinOp::Shl | BinOp::LShr | BinOp::AShr =>
                        Err("E_UNSUPPORTED_NATIVE_BITWISE: checked bitwise and shift lowering is not yet available".into()),
                    BinOp::And | BinOp::Or => {
                        if ty != "i1" {
                            return Err("E_UNSUPPORTED_NATIVE_LOGICAL: expected bool".into());
                        }
                        let tmp = self.id("v");
                        let opcode = if *op == BinOp::And { "and" } else { "or" };
                        self.instr(format!("%{tmp} = {opcode} i1 {a}, {b}"));
                        Ok(format!("%{tmp}"))
                    }
                    BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                        let pred = match op {
                            BinOp::Eq => "eq", BinOp::Ne => "ne",
                            BinOp::Lt => "slt", BinOp::Le => "sle",
                            BinOp::Gt => "sgt", _ => "sge",
                        };
                        let tmp = self.id("v");
                        self.instr(format!("%{tmp} = icmp {pred} {ty} {a}, {b}"));
                        Ok(format!("%{tmp}"))
                    }
                }
            }
            TExpr::Call(index, args) => {
                let fun = self.program.funcs.get(*index as usize)
                    .ok_or("E_INTERNAL: bad function index")?;
                if args.len() != fun.params.len() {
                    return Err("E_INTERNAL: checked call arity mismatch".into());
                }
                let mut pieces = Vec::new();
                for (arg, pty) in args.iter().zip(fun.params.iter()) {
                    let v = self.expr(arg)?;
                    let ty = scalar_type(*pty)?;
                    pieces.push(format!("{ty} {v}"));
                }
                let rty = scalar_type(fun.ret)?;
                if rty == "void" {
                    self.instr(format!("call void @atto_func_{}({})", index, pieces.join(", ")));
                    Ok(String::new())
                } else {
                    let tmp = self.id("v");
                    self.instr(format!("%{tmp} = call {rty} @atto_func_{}({})", index, pieces.join(", ")));
                    Ok(format!("%{tmp}"))
                }
            }
            TExpr::Spawn(actor_type) => {
                if self.program.actors.is_empty() {
                    return Err("E_UNSUPPORTED_NATIVE_ACTOR: no actor table".into());
                }
                let result = self.id("actor_ref");
                self.instr(format!("%{result} = call i32 @atto_actor_spawn(i32 {actor_type})"));
                self.checked_actor_result(&format!("%{result}"), "i32", "0");
                Ok(format!("%{result}"))
            }
            TExpr::Send { target, value } => {
                let target_ref = self.expr(target)?;
                let payload = self.expr(value)?;
                let promoted = match value.ty {
                    Type::Int => payload,
                    Type::Bool => {
                        let widened = self.id("mail_bool");
                        self.instr(format!("%{widened} = zext i1 {payload} to i64"));
                        format!("%{widened}")
                    }
                    _ => return Err("E_UNSUPPORTED_NATIVE_MAIL: only int/bool".into()),
                };
                let status = self.id("send_status");
                self.instr(format!(
                    "%{status} = call i32 @atto_actor_send(i32 {target_ref}, i64 {promoted})"
                ));
                self.checked_actor_result(&format!("%{status}"), "i32", "-1");
                // A full mailbox (-2) also fails closed; no silently lost mail.
                let full = self.id("send_full");
                let trapped = self.id("send_trap");
                let accepted = self.id("send_ok");
                self.instr(format!("%{full} = icmp ne i32 %{status}, 0"));
                self.instr(format!("br i1 %{full}, label %{trapped}, label %{accepted}"));
                self.terminated = true;
                self.label(&trapped);
                self.instr("call void @llvm.trap()");
                self.instr("unreachable");
                self.terminated = true;
                self.label(&accepted);
                Ok(String::new())
            }
            TExpr::GlobalGet(slot) => {
                let typ = *self.program.globals.get(*slot as usize)
                    .ok_or("E_INTERNAL: actor state slot outside typed layout")?;
                let raw = self.id("state");
                self.instr(format!("%{raw} = call i64 @atto_actor_state_load(i32 {slot})"));
                match typ {
                    Type::Int => Ok(format!("%{raw}")),
                    Type::Bool => {
                        let boolean = self.id("state_bool");
                        self.instr(format!("%{boolean} = icmp ne i64 %{raw}, 0"));
                        Ok(format!("%{boolean}"))
                    }
                    _ => Err("E_UNSUPPORTED_NATIVE_STATE: only int/bool fields".into()),
                }
            }
            TExpr::Print { value, newline } => {
                if self.program.actors.is_empty() {
                    return Err("E_UNSUPPORTED_NATIVE_PRINT: requires actor bootstrap".into());
                }
                let v = self.expr(value)?;
                let as_int = match value.ty {
                    Type::Int => v,
                    Type::Bool => {
                        let tmp = self.id("print_bool");
                        self.instr(format!("%{tmp} = zext i1 {v} to i64"));
                        format!("%{tmp}")
                    }
                    _ => return Err("E_UNSUPPORTED_NATIVE_PRINT: only int/bool".into()),
                };
                self.instr(format!(
                    "call void @atto_actor_print_i64(i64 {as_int}, i32 {})",
                    i32::from(*newline)
                ));
                Ok(String::new())
            }
            TExpr::Ternary(cond, yes, no) => {
                let cv = self.expr(cond)?;
                let yes_label = self.id("tern_yes");
                let no_label = self.id("tern_no");
                let done_label = self.id("tern_end");
                self.instr(format!("br i1 {cv}, label %{yes_label}, label %{no_label}"));
                self.terminated = true;
                self.label(&yes_label);
                let y = self.expr(yes)?;
                let y_pred = self.block_name.clone();
                self.branch(&done_label);
                self.label(&no_label);
                let n = self.expr(no)?;
                let n_pred = self.block_name.clone();
                self.branch(&done_label);
                self.label(&done_label);
                let ty = scalar_type(yes.ty)?;
                if ty == "void" {
                    return Err("E_UNSUPPORTED_NATIVE_TERNARY: void".into());
                }
                let tmp = self.id("v");
                self.instr(format!("%{tmp} = phi {ty} [ {y}, %{y_pred} ], [ {n}, %{n_pred} ]"));
                Ok(format!("%{tmp}"))
            }
            _ => Err(format!("E_UNSUPPORTED_NATIVE_EXPR: {:?}", expr.e)),
        }
    }

    fn block(&mut self, statements: &[TStmt]) -> Result<(), String> {
        for statement in statements {
            if self.terminated {
                break;
            }
            self.statement(statement)?;
        }
        Ok(())
    }

    fn statement(&mut self, statement: &TStmt) -> Result<(), String> {
        match statement {
            TStmt::Set(slot, value) => {
                let idx = *slot as usize;
                let ty = scalar_type(*self.slots.get(idx).ok_or("E_INTERNAL: bad set slot")?)?;
                let x = self.expr(value)?;
                self.instr(format!("store {ty} {x}, ptr %local{idx}"));
            }
            TStmt::GlobalSet(slot, value) => {
                let typ = *self.program.globals.get(*slot as usize)
                    .ok_or("E_INTERNAL: actor state index invalid")?;
                let v = self.expr(value)?;
                let as_int = match typ {
                    Type::Int => v,
                    Type::Bool => {
                        let tmp = self.id("store_bool");
                        self.instr(format!("%{tmp} = zext i1 {v} to i64"));
                        format!("%{tmp}")
                    }
                    _ => return Err("E_UNSUPPORTED_NATIVE_STATE: only int/bool".into()),
                };
                self.instr(format!(
                    "call void @atto_actor_state_store(i32 {slot}, i64 {as_int})"
                ));
            }
            TStmt::Expr(expr) => {
                let _ = self.expr(expr)?;
            }
            TStmt::Return(Some(expr)) => {
                let ty = scalar_type(expr.ty)?;
                if ty == "void" {
                    return Err("E_INTERNAL: return void value".into());
                }
                let x = self.expr(expr)?;
                self.instr(format!("ret {ty} {x}"));
                self.terminated = true;
            }
            TStmt::Return(None) => {
                if self.func.ret != Type::Void {
                    return Err("E_INTERNAL: return without value".into());
                }
                self.instr("ret void");
                self.terminated = true;
            }
            TStmt::Block(block) => self.block(block)?,
            TStmt::If(arms, fallback) => {
                let end_label = self.id("if_end");
                for (test, body) in arms {
                    let cv = self.expr(test)?;
                    let then_label = self.id("if_then");
                    let next_label = self.id("if_next");
                    self.instr(format!("br i1 {cv}, label %{then_label}, label %{next_label}"));
                    self.terminated = true;
                    self.label(&then_label);
                    self.block(body)?;
                    if !self.terminated {
                        self.branch(&end_label);
                    }
                    self.label(&next_label);
                }
                if let Some(otherwise) = fallback {
                    self.block(otherwise)?;
                }
                if !self.terminated {
                    self.branch(&end_label);
                }
                self.label(&end_label);
            }
            _ => return Err(format!("E_UNSUPPORTED_NATIVE_STMT: {statement:?}")),
        }
        Ok(())
    }

    fn emit(mut self, index: usize) -> Result<String, String> {
        let return_ty = scalar_type(self.func.ret)?;
        let mut args = Vec::new();
        for (i, t) in self.func.params.iter().enumerate() {
            args.push(format!("{} %arg{i}", scalar_type(*t)?));
        }
        let _ = writeln!(self.out, "define internal {return_ty} @atto_func_{index}({}) {{", args.join(", "));
        self.label("entry");
        let all_slots = self.slots.clone();
        for (idx, ty) in all_slots.into_iter().enumerate() {
            let llty = scalar_type(ty)?;
            self.instr(format!("%local{idx} = alloca {llty}"));
            if idx < self.func.params.len() {
                self.instr(format!("store {llty} %arg{idx}, ptr %local{idx}"));
            } else {
                let zero = if ty == Type::Bool { "false" } else { "0" };
                self.instr(format!("store {llty} {zero}, ptr %local{idx}"));
            }
        }
        self.block(&self.func.body)?;
        if !self.terminated {
            // The typed checker must prove definite return on reachable paths.
            // An unreachable join block traps rather than inventing a result.
            self.instr("call void @llvm.trap()");
            self.instr("unreachable");
            self.terminated = true;
        }
        let _ = writeln!(self.out, "}}");
        Ok(self.out)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LibraryProfile {
    Hosted,
    FreestandingCore,
}

// Admission is performed on the *actual compiler tokens*, not on regexes
// over raw source. Comments and literals cannot smuggle imports or produce
// false host-API matches. This scalar-first profile deliberately denies all
// source imports until the typed module resolver enforces layer provenance.
fn admit_freestanding_tokens(
    tokens: &[atto_compiler::lexer::Token],
) -> Result<(), String> {
    use atto_compiler::lexer::Tok;
    for (index, token) in tokens.iter().enumerate() {
        let Tok::Ident(word) = &token.tok else { continue };
        match word.as_str() {
            "import" | "using" => {
                return Err(format!("E_FREESTANDING_FORBIDDEN_IMPORT: {word} is not admitted"));
            }
            "extern" | "foreign" | "ffi" | "syscall" | "dlopen" => {
                return Err(format!("E_FREESTANDING_HOST_EFFECT: {word}"));
            }
            "new" => return Err("E_ALLOCATOR_REQUIRED: 'new' in core-only profile".into()),
            "Array" | "List" | "Map" | "Set" | "Box" | "Arc" | "String" => {
                return Err(format!("E_ALLOCATOR_REQUIRED: heap-backed {word} in core-only profile"));
            }
            "stdio" | "process" | "filesystem" | "socket" | "thread" | "std" |
            "wasi" | "libc" | "java" | "dotnet" | "System" | "net" | "os" => {
                if matches!(tokens.get(index + 1).map(|x| &x.tok),
                            Some(Tok::Dot | Tok::ColonColon)) {
                    return Err(format!("E_FREESTANDING_HOST_EFFECT: {word} qualified host API"));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

pub fn compile_ir(source: &str) -> Result<String, String> {
    compile_ir_with_profile(source, LibraryProfile::Hosted)
}

fn compile_ir_with_profile(source: &str, profile: LibraryProfile) -> Result<String, String> {
    if source.len() > atto_compiler::SOURCE_CAP {
        return Err("E_SOURCE_LIMIT: native Atto input exceeds 64 KiB".into());
    }
    let toks = atto_compiler::lexer::lex(source).map_err(|e|e.to_string())?;
    if profile == LibraryProfile::FreestandingCore {
        admit_freestanding_tokens(&toks)?;
    }
    let program = if target_native::requires_target_link(&toks) {
        target_native::lower(&toks)?
    } else {
        atto_compiler::parser::Parser::new(toks).parse_program()
            .map_err(|e|e.to_string())?
    };
    let typed = check::check(&program).map_err(|e|e.to_string())?;
    if profile == LibraryProfile::FreestandingCore {
        if !typed.imports.is_empty() {
            return Err("E_FREESTANDING_FORBIDDEN_IMPORT: typed host imports".into());
        }
        if typed.heap || typed.strings || typed.guest_eh || !typed.data.is_empty() {
            return Err("E_ALLOCATOR_REQUIRED: non-scalar runtime payload in core-only profile".into());
        }
        if !typed.actors.is_empty() || !typed.globals.is_empty() ||
           !typed.generators.is_empty() {
            return Err("E_FREESTANDING_RUNTIME: implicit actor/executor/global runtime".into());
        }
    }
    let native_actor_mode = !typed.actors.is_empty();
    if !typed.generators.is_empty() || typed.heap || typed.strings ||
       typed.guest_eh || !typed.data.is_empty() {
        return Err("E_UNSUPPORTED_NATIVE_RUNTIME: guest object heap/string/generator/exception not lowered".into());
    }
    if typed.actors.len() > 32 || typed.globals.len() > 1024 {
        return Err("E_NATIVE_ACTOR_LIMIT: 32 actors, 1024 state slots".into());
    }
    if !native_actor_mode && (!typed.imports.is_empty() || !typed.globals.is_empty()) {
        return Err("E_UNSUPPORTED_NATIVE_RUNTIME: standalone I/O and actor state require actor runtime".into());
    }
    for import in &typed.imports {
        if !native_actor_mode || !matches!(import,
            Import::ActorSpawn | Import::ActorSend | Import::PrintI64 |
            Import::PrintBool | Import::PrintNewline
        ) {
            return Err(format!("E_UNSUPPORTED_NATIVE_IMPORT: {import:?}"));
        }
    }
    for typ in &typed.globals {
        if !matches!(typ, Type::Int | Type::Bool) {
            return Err("E_UNSUPPORTED_NATIVE_STATE: only int/bool actor slots".into());
        }
    }
    for actor in &typed.actors {
        if actor.receive.is_none() {
            return Err("E_UNSUPPORTED_NATIVE_ACTOR: every actor requires receive".into());
        }
    }
    let main = typed.funcs.get(typed.main as usize)
        .ok_or("E_INTERNAL: main index not found")?;
    if !main.params.is_empty() || main.ret != Type::Int {
        return Err("E_UNSUPPORTED_NATIVE_MAIN: requires main(): int".into());
    }
    let mut out = String::from(
        "; Atto scalar native AOT: checked typed frontend, direct LLVM IR, no guest interpreter\n\
         declare void @llvm.trap()\n\
         declare { i64, i1 } @llvm.sadd.with.overflow.i64(i64, i64)\n\
         declare { i64, i1 } @llvm.ssub.with.overflow.i64(i64, i64)\n\
         declare { i64, i1 } @llvm.smul.with.overflow.i64(i64, i64)\n\n");
    if native_actor_mode {
        out.push_str(
            "; Actor-owned C ABI with pthread per-actor stack/arena: no guest GC.\n\
             declare i32 @atto_actor_spawn(i32)\n\
             declare i32 @atto_actor_send(i32, i64)\n\
             declare i64 @atto_actor_state_load(i32)\n\
             declare void @atto_actor_state_store(i32, i64)\n\
             declare void @atto_actor_print_i64(i64, i32)\n\
             declare void @atto_actor_finish()\n\n"
        );
    }
    for (idx, func) in typed.funcs.iter().enumerate() {
        out.push_str(&FunctionEmitter::new(&typed, func)?.emit(idx)?);
        out.push('\n');
    }
    if native_actor_mode {
        // Each actor receives a fresh instance arena and its state initializers
        // run on its own pthread stack. No static LLVM global stores exist.
        for (idx, actor) in typed.actors.iter().enumerate() {
            let mut init = TFunc {
                name: format!("$actor_init_{idx}"),
                params: Vec::new(), ret: Type::Void, locals: Vec::new(),
                body: actor.inits.iter().map(|(slot, value)|
                    TStmt::GlobalSet(*slot, value.clone())).collect(),
            };
            init.body.push(TStmt::Return(None));
            out.push_str(&FunctionEmitter::new(&typed, &init)?
                .emit(typed.funcs.len() + idx)?);
            out.push('\n');
        }
        let _ = writeln!(out, "define void @atto_actor_init(i32 %kind) {{\nentry:");
        let _ = writeln!(out, "  switch i32 %kind, label %init_invalid [");
        for ai in 0..typed.actors.len() {
            let _ = writeln!(out, "    i32 {ai}, label %init_{ai}");
        }
        let _ = writeln!(out, "  ]");
        for ai in 0..typed.actors.len() {
            let _ = writeln!(out,
                "init_{ai}:\n  call void @atto_func_{}()\n  ret void",
                typed.funcs.len() + ai
            );
        }
        out.push_str("init_invalid:\n  call void @llvm.trap()\n  unreachable\n}\n");
        out.push_str("define void @atto_actor_receive(i32 %kind, i64 %value, i64 %sequence) {\nentry:\n");
        out.push_str("  switch i32 %kind, label %recv_invalid [\n");
        for ai in 0..typed.actors.len() {
            let _ = writeln!(out, "    i32 {ai}, label %recv_{ai}");
        }
        out.push_str("  ]\n");
        for (ai, actor) in typed.actors.iter().enumerate() {
            let rx_idx = actor.receive
                .ok_or("E_UNSUPPORTED_NATIVE_ACTOR: no receive handler")?;
            let func = typed.funcs.get(rx_idx as usize)
                .ok_or("E_INTERNAL: invalid receive function index")?;
            if func.params != [Type::Int, Type::Int] || func.ret != Type::Void {
                return Err("E_UNSUPPORTED_NATIVE_ACTOR: receive ABI mismatch".into());
            }
            let _ = writeln!(out,
                "recv_{ai}:\n  call void @atto_func_{rx_idx}(i64 %value, i64 %sequence)\n  ret void"
            );
        }
        out.push_str("recv_invalid:\n  call void @llvm.trap()\n  unreachable\n}\n");
    }
    let finish = if native_actor_mode { "  call void @atto_actor_finish()\n" }
        else { "" };
    let _ = writeln!(out,
        "define i32 @main() {{\nentry:\n  %answer = call i64 @atto_func_{}()\n{}  %code = trunc i64 %answer to i32\n  ret i32 %code\n}}\n",
        typed.main, finish
    );
    Ok(out)
}

// Compile outputs are staged in the destination directory and atomically
// published only after a complete IR write / successful clang exit. This
// prevents compiler failures from truncating a previous valid artifact.
// Creating a private temp path with create_new also rejects preexisting
// symlinks rather than following them.
static OUTPUT_SEQUENCE: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

struct OutputStage {
    temp: std::path::PathBuf,
    destination: std::path::PathBuf,
}

impl OutputStage {
    fn create(
        source: &std::path::Path,
        destination: &std::path::Path,
    ) -> Result<(Self, std::fs::File), String> {
        if let Ok(output_meta) = std::fs::metadata(destination) {
            let source_meta = std::fs::metadata(source)
                .map_err(|e| format!("E_INPUT: {e}"))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if source_meta.dev() == output_meta.dev()
                    && source_meta.ino() == output_meta.ino()
                {
                    return Err("E_OUTPUT_SOURCE: output aliases the input source".into());
                }
            }
            if std::fs::canonicalize(destination).ok()
                == std::fs::canonicalize(source).ok()
            {
                return Err("E_OUTPUT_SOURCE: output aliases the input source".into());
            }
        }
        let parent = destination.parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| std::path::Path::new("."));
        let name = destination.file_name()
            .ok_or("E_OUTPUT: missing output filename")?
            .to_string_lossy();
        for _ in 0..64 {
            let sequence = OUTPUT_SEQUENCE.fetch_add(
                1, std::sync::atomic::Ordering::Relaxed
            );
            let temp = parent.join(format!(
                ".{name}.atto-stage-{}-{sequence}.tmp",
                std::process::id()
            ));
            match std::fs::OpenOptions::new()
                .write(true).create_new(true).open(&temp)
            {
                Ok(file) => return Ok((
                    Self { temp, destination: destination.to_path_buf() },
                    file
                )),
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(err) => return Err(format!("E_OUTPUT: {err}")),
            }
        }
        Err("E_OUTPUT: could not allocate unique staging filename".into())
    }

    fn publish(self) -> Result<(), String> {
        std::fs::rename(&self.temp, &self.destination)
            .map_err(|e| format!("E_OUTPUT_PUBLISH: {e}"))
    }
}

impl Drop for OutputStage {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.temp);
    }
}

// A zero-exit wrapper is not enough to prove clang wrote a valid native
// artifact: validate the admitted Linux/x86-64 ELF output before publication.
fn validate_native_artifact(path: &std::path::Path) -> Result<(), String> {
    let mut file = std::fs::File::open(path)
        .map_err(|e| format!("E_CLANG_ARTIFACT: {e}"))?;
    let mut header = [0u8; 20];
    file.read_exact(&mut header)
        .map_err(|e| format!("E_CLANG_ARTIFACT: incomplete ELF header: {e}"))?;
    if header[..4] != [0x7f, b'E', b'L', b'F']
        || header[4] != 2   // ELFCLASS64
        || header[5] != 1   // little endian
        || header[18..20] != [62, 0]  // EM_X86_64
    {
        return Err("E_CLANG_ARTIFACT: expected Linux x86-64 ELF64 output".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if file.metadata()
            .map_err(|e| format!("E_CLANG_ARTIFACT: {e}"))?
            .permissions().mode() & 0o111 == 0
        {
            return Err("E_CLANG_ARTIFACT: generated ELF is not executable".into());
        }
    }
    Ok(())
}

fn cli() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (src, out, emit_only, profile) = match args.as_slice() {
        [src, flag, out] if flag == "-o" => (src, out, false, LibraryProfile::Hosted),
        [src, flag, out, mode] if flag == "-o" && mode == "--emit-llvm" =>
            (src, out, true, LibraryProfile::Hosted),
        [src, flag, out, mode, profile]
            if flag == "-o" && mode == "--emit-llvm" && profile == "--freestanding-core" =>
            (src, out, true, LibraryProfile::FreestandingCore),
        _ => return Err(
            "usage: atto-llvm input.atto -o output [--emit-llvm [--freestanding-core]]".into()
        ),
    };
    if !src.ends_with(".atto") {
        return Err("E_SOURCE: input must have the .atto extension".into());
    }
    let content = std::fs::read_to_string(src).map_err(|e|format!("E_INPUT: {e}"))?;
    let ir = compile_ir_with_profile(&content, profile)?;
    let (staged, mut stage_file) = OutputStage::create(
        std::path::Path::new(src), std::path::Path::new(out)
    )?;
    if emit_only {
        stage_file.write_all(ir.as_bytes()).map_err(|e|format!("E_OUTPUT: {e}"))?;
        stage_file.flush().map_err(|e|format!("E_OUTPUT: {e}"))?;
        drop(stage_file);
        return staged.publish();
    }
    drop(stage_file);
    // LLVM's clang lowers IR to closed-world machine code in a private staging
    // path; even compiler failure cannot damage the previous output artifact.
    // No Wasm runtime, guest interpreter, dynamic language VM, or collector.
    let native_actor_mode = ir.contains("define void @atto_actor_init(i32 %kind)");
    let mut compiler = Command::new("clang");
    compiler.args(["-x", "ir", "-O2", "-o"])
        .arg(&staged.temp)
        .arg("-");
    if native_actor_mode {
        let runtime = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("runtime/actor_runtime.c");
        if !runtime.is_file() {
            return Err("E_NATIVE_ACTOR_RUNTIME: native C actor runtime missing".into());
        }
        compiler.args(["-x", "c"]).arg(runtime).arg("-pthread");
    }
    let mut child = compiler.stdin(Stdio::piped()).spawn()
        .map_err(|e|format!("E_CLANG: {e}"))?;
    let write_result = match child.stdin.take() {
        Some(mut input) => input.write_all(ir.as_bytes()),
        None => Err(std::io::Error::other("clang stdin unavailable")),
    };
    if let Err(err) = write_result {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!("E_CLANG_INPUT: {err}"));
    }
    let status = child.wait().map_err(|e|format!("E_CLANG_WAIT: {e}"))?;
    if !status.success() {
        return Err(format!("E_CLANG: compilation failed ({status})"));
    }
    validate_native_artifact(&staged.temp)?;
    staged.publish()
}

fn main() {
    if let Err(e) = cli() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_main_is_direct_i64_call() {
        let ir = compile_ir("pub fnc main(): int { return 42; }").unwrap();
        assert!(ir.contains("define i32 @main()"));
        assert!(ir.contains("ret i64 42"));
        assert!(!ir.contains("wasm"));
    }
    #[test]
    fn rejects_unbound_identifiers_without_emitting_ir() {
        let e = compile_ir("pub fnc main(): int { return missing_symbol; }").unwrap_err();
        assert!(e.contains("E_UNBOUND"), "{e}");
    }
    #[test]
    fn retains_checked_overflow_intrinsics() {
        let ir = compile_ir("pub fnc main(): int { return 2 + 3; }").unwrap();
        assert!(ir.contains("llvm.sadd.with.overflow.i64"));
        assert!(ir.contains("llvm.trap"));
    }
    #[test]
    fn rejects_runtime_printing_without_native_abi() {
        assert!(compile_ir("pub fnc main(): int { stdio.println(1); return 42; }").is_err());
    }
    #[test]
    fn freestanding_core_compiles_checked_scalar_without_host_runtime() {
        let ir = compile_ir_with_profile(
            "pub fnc add_one(int x): int { return x + 1; } pub fnc main(): int { return add_one(41); }",
            LibraryProfile::FreestandingCore,
        ).unwrap();
        assert!(ir.contains("define i32 @main()"));
        assert!(ir.contains("llvm.sadd.with.overflow.i64"));
        assert!(!ir.contains("@atto_actor_"));
        assert!(!ir.contains("@malloc"));
    }
    #[test]
    fn freestanding_core_ignores_host_names_in_comments() {
        let source = "// import std; stdio.out; new Array<int>;\npub fnc main(): int { return 42; }";
        assert!(compile_ir_with_profile(source, LibraryProfile::FreestandingCore).is_ok());
    }
    #[test]
    fn freestanding_core_rejects_imports_and_host_dependencies() {
        let samples = [
            ("import module host from \"stdlib/std/file.atto\"; pub fnc main(): int { return 42; }", "E_FREESTANDING_FORBIDDEN_IMPORT"),
            ("pub fnc main(): int { stdio.println(1); return 42; }", "E_FREESTANDING_HOST_EFFECT"),
            ("pub fnc main(): int { process.spawn(\"a\"); return 42; }", "E_FREESTANDING_HOST_EFFECT"),
            ("pub fnc main(): int { val x = Array.of(1); return 42; }", "E_ALLOCATOR_REQUIRED"),
            ("pub fnc main(): int { val x = new Error(\"x\"); return 42; }", "E_ALLOCATOR_REQUIRED"),
        ];
        for (source, code) in samples {
            let err = compile_ir_with_profile(source, LibraryProfile::FreestandingCore).unwrap_err();
            assert!(err.contains(code), "{code} got {err}");
        }
    }
}
