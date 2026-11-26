//! Bounded checked scalar ESM target. No heap, actors, arbitrary foreign imports,
//! exception handling, or continuation operations may escape this profile.
use crate::ast::{BinOp, Type};
use crate::check::{Builtin, Import, TExpr, TProgram, TStmt, TE};
use crate::{Diagnostic, Pos};

pub(crate) fn unsupported() -> Diagnostic {
    Diagnostic::new(Pos { line: 1, col: 1 }, "E_JS_UNSUPPORTED",
        "ESM target currently admits only checked scalar int/bool/float/void IR and standard output")
}
type R<T> = Result<T, Diagnostic>;
fn scalar(t: Type) -> bool { matches!(t, Type::Int | Type::Bool | Type::Float | Type::Void) }
fn expr(e: &TE) -> R<String> {
    if !scalar(e.ty) { return Err(unsupported()); }
    Ok(match &e.e {
        TExpr::I64(n) => format!("({n}n)"),
        TExpr::F64(n) => if n.is_nan() { "NaN".into() } else if *n == f64::INFINITY { "Infinity".into() } else if *n == f64::NEG_INFINITY { "(-Infinity)".into() } else { format!("({n:?})") },
        TExpr::IntToFloat(a) => format!("Number({})", expr(a)?),
        TExpr::FNeg(a) => format!("(-({}))", expr(a)?),
        TExpr::Bool(b) => if *b { "1".into() } else { "0".into() },
        TExpr::Local(i) => format!("l{i}"),
        TExpr::Neg(a) => format!("checked(-({}))", expr(a)?),
        TExpr::Not(a) => format!("Number(!({}))", expr(a)?),
        TExpr::Bin(op, a, b) => {
            let l = expr(a)?;
            let r = expr(b)?;
            match op {
                BinOp::Add | BinOp::Sub | BinOp::Mul => {
                    let op = match op { BinOp::Add => "+", BinOp::Sub => "-", _ => "*" };
                    if e.ty == Type::Float { format!("(({l}){op}({r}))") } else { format!("checked(({l}){op}({r}))") }
                }
                BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                    let op = match op { BinOp::Eq => "===", BinOp::Ne => "!==",
                        BinOp::Lt => "<", BinOp::Le => "<=", BinOp::Gt => ">", _ => ">=" };
                    format!("Number(({l}){op}({r}))")
                }
                BinOp::And | BinOp::Or => {
                    let op = if matches!(op, BinOp::And) { "&" } else { "|" };
                    format!("(({l}){op}({r}))")
                }
                BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor => {
                    let op = match op { BinOp::BitAnd => "&", BinOp::BitOr => "|", _ => "^" };
                    format!("BigInt.asIntN(64,({l}){op}({r}))")
                }
                BinOp::Shl | BinOp::LShr | BinOp::AShr => {
                    let op = match op { BinOp::Shl => 0, BinOp::LShr => 1, _ => 2 };
                    format!("shift({l},{r},{op})")
                }
                BinOp::Div if a.ty == Type::Float && b.ty == Type::Float => format!("(({l})/({r}))"),
                BinOp::Div => return Err(unsupported()),
            }
        }
        TExpr::Ternary(c, a, b) => format!("(({})?({}):({}))", expr(c)?, expr(a)?, expr(b)?),
        TExpr::Call(i, args) => format!("f{i}({})",
            args.iter().map(expr).collect::<R<Vec<_>>>()?.join(",")),
        TExpr::Builtin(op, a, b) => {
            let op = match op { Builtin::And => "&", Builtin::Or => "|", Builtin::Xor => "^" };
            format!("(({}){op}({}))", expr(a)?, expr(b)?)
        }
        TExpr::Print { value, newline } => {
            let name = match value.ty { Type::Int => "print_i64", Type::Bool => "print_bool", Type::Float => "print_f64", _ => return Err(unsupported()) };
            let call = format!("host_{name}({})", expr(value)?);
            if *newline { format!("({call},host_print_newline())") } else { call }
        }
        TExpr::PrintStr { offset, len, newline } => {
            let call = format!("host_print_str({offset},{len})");
            if *newline { format!("({call},host_print_newline())") } else { call }
        }
        _ => return Err(unsupported()),
    })
}

fn stmts(body: &[TStmt], out: &mut String, void_main: bool, loops: &mut Vec<usize>, next: &mut usize) -> R<()> {
    use std::fmt::Write;
    for s in body {
        match s {
            TStmt::Set(i, v) => writeln!(out, "l{i}={};", expr(v)?).unwrap(),
            TStmt::Expr(e) => writeln!(out, "{};", expr(e)?).unwrap(),
            TStmt::Return(v) => {
                let value = match v { Some(e) => expr(e)?, None if void_main => "0n".into(),
                    None => "undefined".into() };
                writeln!(out, "return {value};").unwrap();
            }
            TStmt::If(arms, other) => {
                for (i, (cond, body)) in arms.iter().enumerate() {
                    writeln!(out, "{}if({}){{", if i == 0 { "" } else { "else " }, expr(cond)?).unwrap();
                    stmts(body, out, void_main, loops, next)?;
                    out.push_str("}\n");
                }
                if let Some(body) = other {
                    out.push_str("else{\n");
                    stmts(body, out, void_main, loops, next)?;
                    out.push_str("}\n");
                }
            }
            TStmt::Loop { cond, body, update } => {
                let id = *next;
                *next += 1;
                let cond = match cond { Some(c) => expr(c)?, None => "1".into() };
                writeln!(out, "loop{id}:while({cond}){{body{id}:{{").unwrap();
                loops.push(id);
                stmts(body, out, void_main, loops, next)?;
                loops.pop();
                out.push_str("}\n");
                // A source continue leaves the body block and executes the update.
                // Update expressions cannot acquire a new break/continue target.
                stmts(update, out, void_main, &mut Vec::new(), next)?;
                out.push_str("}\n");
            }
            TStmt::Break | TStmt::Continue => {
                let id = loops.last().ok_or_else(unsupported)?;
                let label = if matches!(s, TStmt::Break) { "loop" } else { "body" };
                writeln!(out, "break {label}{id};").unwrap();
            }
            TStmt::Block(body) => {
                out.push_str("{\n");
                stmts(body, out, void_main, loops, next)?;
                out.push_str("}\n");
            }
            _ => return Err(unsupported()),
        }
        if out.len() > crate::wasm::MAX_OUTPUT { return Err(unsupported()); }
    }
    Ok(())
}

pub(crate) fn emit(p: &TProgram) -> R<String> {
    use std::fmt::Write;
    if p.heap || p.strings || p.guest_eh || p.raise_tag || p.catch_depth ||
        !p.actors.is_empty() || !p.generators.is_empty() || !p.globals.is_empty() ||
        p.imports.iter().any(|i| !matches!(i, Import::PrintI64 | Import::PrintBool | Import::PrintF64 | Import::PrintStr | Import::PrintNewline)) || p.data.len() > 65536 {
        return Err(unsupported());
    }
    let main = p.funcs.get(p.main as usize).ok_or_else(unsupported)?;
    if !main.params.is_empty() || !matches!(main.ret, Type::Int | Type::Void) {
        return Err(unsupported());
    }
    let mut out = String::from("export function instantiate(atto={}){\nconst panic_code={value:0};\nfunction checked(n){if(n<-(1n<<63n)||n>(1n<<63n)-1n){panic_code.value=1;throw new Error('E_PANIC:1')}return n}\n");
    out.push_str("function shift(a,n,op){if(n<0n||n>=64n){panic_code.value=8;throw new Error('E_PANIC:8')}return BigInt.asIntN(64,op===0?a<<n:op===1?BigInt.asUintN(64,a)>>n:a>>n)}\n");
    for import in &p.imports {
        let name = import.name();
        writeln!(out, "const host_{name}=atto.{name};if(typeof host_{name}!=='function')throw new WebAssembly.LinkError('missing import atto.{name}');").unwrap();
    }
    let has_memory = !p.data.is_empty() || p.imports.contains(&Import::PrintStr);
    if has_memory {
        out.push_str("const memory=new WebAssembly.Memory({initial:1,maximum:1});\n");
        writeln!(out, "new Uint8Array(memory.buffer).set([{}]);", p.data.iter().map(u8::to_string).collect::<Vec<_>>().join(",")).unwrap();
    }
    for (i, f) in p.funcs.iter().enumerate() {
        if !scalar(f.ret) || f.params.iter().chain(&f.locals).any(|t| !matches!(t, Type::Int | Type::Bool | Type::Float)) {
            return Err(unsupported());
        }
        let void_main = i == p.main as usize && f.ret == Type::Void;
        let args = (0..f.params.len()).map(|j| format!("l{j}")).collect::<Vec<_>>().join(",");
        writeln!(out, "function f{i}({args}){{").unwrap();
        for (j, t) in f.locals.iter().enumerate() {
            writeln!(out, "let l{}={};", j+f.params.len(), if *t == Type::Int { "0n" } else { "0" }).unwrap();
        }
        stmts(&f.body, &mut out, void_main, &mut Vec::new(), &mut 0)?;
        if void_main { out.push_str("return 0n;\n"); }
        out.push_str("}\n");
    }
    writeln!(out, "return {{exports:{{main:f{},panic_code{}}}}};\n}}", p.main, if has_memory { ",memory" } else { "" }).unwrap();
    out.push_str("export const actorNames=Object.freeze([]);\n");
    if out.len() > crate::wasm::MAX_OUTPUT { return Err(unsupported()); }
    Ok(out)
}
