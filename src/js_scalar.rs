//! Bounded checked scalar ESM target. Only string allocations are admitted; no actors, arbitrary foreign imports,
//! exception handling, or continuation operations may escape this profile.
use crate::ast::{BinOp, Type};
use crate::check::{Builtin, Import, StrOp, TExpr, TProgram, TStmt, TE};
use crate::{Diagnostic, Pos};

pub(crate) fn unsupported() -> Diagnostic {
    Diagnostic::new(Pos { line: 1, col: 1 }, "E_JS_UNSUPPORTED",
        "ESM target currently admits only checked scalar and bounded string IR and standard output")
}
type R<T> = Result<T, Diagnostic>;
fn scalar(t: Type) -> bool { matches!(t, Type::Int | Type::Bool | Type::Float | Type::Str | Type::Void) }
fn expr(e: &TE, recursive: &[bool]) -> R<String> {
    if !scalar(e.ty) { return Err(unsupported()); }
    Ok(match &e.e {
        TExpr::I64(n) => format!("({n}n)"),
        TExpr::F64(n) => if n.is_nan() { "NaN".into() } else if *n == f64::INFINITY { "Infinity".into() } else if *n == f64::NEG_INFINITY { "(-Infinity)".into() } else { format!("({n:?})") },
        TExpr::IntToFloat(a) => format!("Number({})", expr(a, recursive)?),
        TExpr::FNeg(a) => format!("(-({}))", expr(a, recursive)?),
        TExpr::Bool(b) => if *b { "1".into() } else { "0".into() },
        TExpr::StrConst(offset, len) => format!("pack({offset},{len})"),
        TExpr::Str(op, a, arg) => {
            let name = match op {
                StrOp::Eq => "str_eq", StrOp::StartsWith => "str_starts",
                StrOp::EndsWith => "str_ends", StrOp::Contains => "str_contains",
                StrOp::Trim => "str_trim", StrOp::Concat => "str_concat", StrOp::Cmp => "str_cmp",
            };
            let lhs = expr(a, recursive)?;
            match arg { Some(b) => format!("{name}({lhs},{})", expr(b, recursive)?), None => format!("{name}({lhs})") }
        }
        TExpr::PrintStrVal { value, newline, .. } => {
            let call = format!("print_value({})", expr(value, recursive)?);
            if *newline { format!("({call},host_print_newline())") } else { call }
        }
        TExpr::Local(i) => format!("l{i}"),
        TExpr::Neg(a) => format!("checked(-({}))", expr(a, recursive)?),
        TExpr::Not(a) => format!("Number(!({}))", expr(a, recursive)?),
        TExpr::Bin(op, a, b) => {
            let l = expr(a, recursive)?;
            let r = expr(b, recursive)?;
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
        TExpr::Ternary(c, a, b) => format!("(({})?({}):({}))", expr(c, recursive)?, expr(a, recursive)?, expr(b, recursive)?),
        TExpr::Call(i, args) => {
            let prefix = if *recursive.get(*i as usize).ok_or_else(unsupported)? { "r" } else { "f" };
            format!("{prefix}{i}({})", args.iter().map(|arg| expr(arg, recursive)).collect::<R<Vec<_>>>()?.join(","))
        }
        TExpr::Builtin(op, a, b) => {
            let op = match op { Builtin::And => "&", Builtin::Or => "|", Builtin::Xor => "^" };
            format!("(({}){op}({}))", expr(a, recursive)?, expr(b, recursive)?)
        }
        TExpr::Print { value, newline } => {
            let name = match value.ty { Type::Int => "print_i64", Type::Bool => "print_bool", Type::Float => "print_f64", _ => return Err(unsupported()) };
            let call = format!("host_{name}({})", expr(value, recursive)?);
            if *newline { format!("({call},host_print_newline())") } else { call }
        }
        TExpr::PrintStr { offset, len, newline } => {
            let call = format!("host_print_str({offset},{len})");
            if *newline { format!("({call},host_print_newline())") } else { call }
        }
        _ => return Err(unsupported()),
    })
}

fn stmts(body: &[TStmt], out: &mut String, void_main: bool, loops: &mut Vec<usize>, next: &mut usize, recursive: &[bool]) -> R<()> {
    use std::fmt::Write;
    for s in body {
        match s {
            TStmt::Set(i, v) => writeln!(out, "l{i}={};", expr(v, recursive)?).unwrap(),
            TStmt::Expr(e) => writeln!(out, "{};", expr(e, recursive)?).unwrap(),
            TStmt::Return(v) => {
                let value = match v { Some(e) => expr(e, recursive)?, None if void_main => "0n".into(),
                    None => "undefined".into() };
                writeln!(out, "return {value};").unwrap();
            }
            TStmt::If(arms, other) => {
                for (i, (cond, body)) in arms.iter().enumerate() {
                    writeln!(out, "{}if({}){{", if i == 0 { "" } else { "else " }, expr(cond, recursive)?).unwrap();
                    stmts(body, out, void_main, loops, next, recursive)?;
                    out.push_str("}\n");
                }
                if let Some(body) = other {
                    out.push_str("else{\n");
                    stmts(body, out, void_main, loops, next, recursive)?;
                    out.push_str("}\n");
                }
            }
            TStmt::Loop { cond, body, update } => {
                let id = *next;
                *next += 1;
                let cond = match cond { Some(c) => expr(c, recursive)?, None => "1".into() };
                writeln!(out, "loop{id}:while({cond}){{body{id}:{{").unwrap();
                loops.push(id);
                stmts(body, out, void_main, loops, next, recursive)?;
                loops.pop();
                out.push_str("}\n");
                // A source continue leaves the body block and executes the update.
                // Update expressions cannot acquire a new break/continue target.
                stmts(update, out, void_main, &mut Vec::new(), next, recursive)?;
                out.push_str("}\n");
            }
            TStmt::Break | TStmt::Continue => {
                let id = loops.last().ok_or_else(unsupported)?;
                let label = if matches!(s, TStmt::Break) { "loop" } else { "body" };
                writeln!(out, "break {label}{id};").unwrap();
            }
            // Exhaustive match lowering includes a terminal impossible arm.
            TStmt::Unreachable => out.push_str("throw new WebAssembly.RuntimeError('unreachable');\n"),
            TStmt::Block(body) => {
                out.push_str("{\n");
                stmts(body, out, void_main, loops, next, recursive)?;
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
    if p.guest_eh || p.raise_tag || p.catch_depth ||
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
    let has_memory = p.heap || p.strings || !p.data.is_empty() || p.imports.contains(&Import::PrintStr);
    if has_memory {
        let pages = if p.heap { crate::wasm::HEAP_PAGES } else { 1 };
        writeln!(out, "const memory=new WebAssembly.Memory({{initial:{pages},maximum:{pages}}});const mem=new Uint8Array(memory.buffer);").unwrap();
        writeln!(out, "new Uint8Array(memory.buffer).set([{}]);", p.data.iter().map(u8::to_string).collect::<Vec<_>>().join(",")).unwrap();
    }
    if p.strings {
        out.push_str(r#"function pack(p,n){return (BigInt(p)<<32n)|BigInt(n)}
function ptr(s){return Number(s>>32n)}
function len(s){return Number(s&0xffffffffn)}
function bytes_eq(p,q,n){for(let i=0;i<n;i++)if(mem[p+i]!==mem[q+i])return 0;return 1}
function str_eq(a,b){return len(a)===len(b)?bytes_eq(ptr(a),ptr(b),len(a)):0}
function str_starts(a,b){return len(a)>=len(b)?bytes_eq(ptr(a),ptr(b),len(b)):0}
function str_ends(a,b){return len(a)>=len(b)?bytes_eq(ptr(a)+len(a)-len(b),ptr(b),len(b)):0}
function str_contains(a,b){let n=len(b),p=ptr(a),q=ptr(b);for(let i=0;i<=len(a)-n;i++)if(bytes_eq(p+i,q,n))return 1;return 0}
function str_trim(a){let p=ptr(a),q=p+len(a);while(p<q&&mem[p]<=32)p++;while(q>p&&mem[q-1]<=32)q--;return pack(p,q-p)}
function str_cmp(a,b){let p=ptr(a),q=ptr(b),n=Math.min(len(a),len(b)),i=0;while(i<n&&mem[p+i]===mem[q+i])i++;if(i===n)return len(a)<len(b)?-1n:len(a)>len(b)?1n:0n;let k=i;while(k>0&&(mem[p+k]&192)===128)k--;let x=mem[p+k],y=mem[q+k],flip=(x>=240&&y>=238&&y<240)||(y>=240&&x>=238&&x<240);return (mem[p+i]<mem[q+i])!==flip?-1n:1n}
function print_value(a){host_print_str(ptr(a),len(a))}
"#);
        if p.heap {
            writeln!(out, "let heap={};", (p.data.len()+7)&!7).unwrap();
            out.push_str(r#"function str_concat(a,b){let n=len(a),m=len(b),size=(n+m+7)&~7;if(size>mem.length-heap){panic_code.value=3;throw new Error('E_PANIC:3')}let p=heap;heap+=size;for(let i=0;i<n;i++)mem[p+i]=mem[ptr(a)+i];for(let i=0;i<m;i++)mem[p+n+i]=mem[ptr(b)+i];return pack(p,n+m)}
"#);
        }
    }
    // Match Wasm's recursive-edge accounting, not total native call depth.
    let n = p.funcs.len();
    let adj: Vec<Vec<usize>> = p.funcs.iter().map(|f| {
        let edges = std::cell::RefCell::new(Vec::new());
        crate::check::any_in_stmts(&f.body, &|e| {
            if let TExpr::Call(target, _) = &e.e { edges.borrow_mut().push(*target as usize); }
            false
        });
        let mut edges = edges.into_inner();
        edges.sort_unstable();
        edges.dedup();
        edges
    }).collect();
    if adj.iter().flatten().any(|&v| v >= n) { return Err(unsupported()); }
    let reach: Vec<Vec<bool>> = (0..n).map(|u| {
        let mut seen = vec![false; n];
        let mut work = adj[u].clone();
        while let Some(v) = work.pop() {
            if !seen[v] { seen[v] = true; work.extend(adj[v].iter().copied()); }
        }
        seen
    }).collect();
    let recursive: Vec<Vec<bool>> = (0..n).map(|u| (0..n).map(|v| reach[v][u] && adj[u].contains(&v)).collect()).collect();
    if recursive.iter().flatten().any(|&r| r) {
        out.push_str("let depth=0;\n");
        for (i, f) in p.funcs.iter().enumerate() {
            if recursive.iter().any(|row| row[i]) {
                let args = (0..f.params.len()).map(|j| format!("a{j}")).collect::<Vec<_>>().join(",");
                // Parameters are evaluated before the edge is charged, as in Wasm.
                // On terminal trap the quota is retained, not silently reset.
                writeln!(out, "function r{i}({args}){{if(depth>={}){{panic_code.value=6;throw new Error('E_PANIC:6')}}depth++;const result=f{i}({args});depth--;return result}}", crate::wasm::MAX_CALL_DEPTH).unwrap();
            }
        }
    }
    for (i, f) in p.funcs.iter().enumerate() {
        if !scalar(f.ret) || f.params.iter().chain(&f.locals).any(|t| !matches!(t, Type::Int | Type::Bool | Type::Float | Type::Str)) {
            return Err(unsupported());
        }
        let void_main = i == p.main as usize && f.ret == Type::Void;
        let args = (0..f.params.len()).map(|j| format!("l{j}")).collect::<Vec<_>>().join(",");
        writeln!(out, "function f{i}({args}){{").unwrap();
        for (j, t) in f.locals.iter().enumerate() {
            writeln!(out, "let l{}={};", j+f.params.len(), if matches!(t, Type::Int | Type::Str) { "0n" } else { "0" }).unwrap();
        }
        stmts(&f.body, &mut out, void_main, &mut Vec::new(), &mut 0, &recursive[i])?;
        if void_main { out.push_str("return 0n;\n"); }
        out.push_str("}\n");
    }
    writeln!(out, "return {{exports:{{main:f{},panic_code{}}}}};\n}}", p.main, if has_memory { ",memory" } else { "" }).unwrap();
    out.push_str("export const actorNames=Object.freeze([]);\n");
    if out.len() > crate::wasm::MAX_OUTPUT { return Err(unsupported()); }
    Ok(out)
}
