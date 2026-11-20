//! Executable, closed-world, single-task Mutex<int> *uncontended fast path*.
//!
//! This admission profile is deliberately narrower than the normative Mutex
//! contract. A source file must have exactly one suspending scalar increment()
//! function and one main(); the mutex is created locally, never escapes, is
//! awaited at most once, and no other task, actor, FFI or handle can run or
//! alias it. The proof permits lock elision on the ready path. The generated
//! Wasm still performs i64 mutation, snapshots the result and clears its
//! private per-instance lease flag BEFORE returning from the helper.
//!
//! General Mutex<T>, contended lock_async, cancellation, exception unwinding,
//! task scheduling and TenantVM transfer have no implementation here. Any
//! deviation returns a diagnostic WITHOUT generating a guest module.
//! This is a typed, deliberately bounded source profile, not textual macro
//! substitution and not a claim of scheduler/async ABI parity.

use crate::ast::{BinOp, Type};
use crate::check::{Import, TE, TExpr, TFunc, TProgram, TStmt};
use crate::diag::{Diagnostic, Pos};
use crate::lexer::{Tok, Token};

type R<T> = Result<T, Diagnostic>;
fn error(pos: Pos, code: &'static str, reason: &str) -> Diagnostic {
    Diagnostic::new(pos, code, reason)
}
struct Cursor<'a> { tokens: &'a [Token], index: usize }
impl<'a> Cursor<'a> {
    fn new(tokens: &'a [Token]) -> Self { Self { tokens, index: 0 } }
    fn at(&self) -> &Tok {
        &self.tokens[self.index.min(self.tokens.len().saturating_sub(1))].tok
    }
    fn pos(&self) -> Pos {
        self.tokens[self.index.min(self.tokens.len().saturating_sub(1))].pos
    }
    fn advance(&mut self) { if !matches!(self.at(), Tok::Eof) { self.index += 1; } }
    fn take(&mut self, want: Tok) -> bool {
        if *self.at() == want { self.advance(); true } else { false }
    }
    fn word(&mut self, want: &str) -> bool {
        if matches!(self.at(), Tok::Ident(name) if name == want) {
            self.advance(); true
        } else { false }
    }
    fn expect(&mut self, want: Tok, desc: &str) -> R<()> {
        if self.take(want) { Ok(()) } else { Err(error(self.pos(), "E_LOCK_UNSUPPORTED", desc)) }
    }
    fn require(&mut self, want: &str) -> R<()> {
        if self.word(want) { Ok(()) } else {
            Err(error(self.pos(), "E_LOCK_UNSUPPORTED", "unsupported mutex fast-path token"))
        }
    }
    fn name(&mut self) -> R<String> {
        match self.at() {
            Tok::Ident(n) if !matches!(n.as_str(),"await"|"return"|"using"|"susp"|"suspends") => {
                let name = n.clone(); self.advance(); Ok(name)
            },
            _ => Err(error(self.pos(), "E_LOCK_UNSUPPORTED", "expected a legal local name")),
        }
    }
    fn int(&mut self) -> R<i64> {
        let pos = self.pos();
        let negative = self.take(Tok::Minus);
        match self.at() {
            Tok::Int(Some(n)) if (!negative && *n <= i64::MAX as u64)
                || (negative && *n <= i64::MAX as u64 + 1) => {
                let n = *n;
                self.advance();
                if negative && n == i64::MAX as u64 + 1 { Ok(i64::MIN) }
                else if negative { Ok(-(n as i64)) } else { Ok(n as i64) }
            },
            _ => Err(error(pos, "E_INTEGER_RANGE", "mutex scalar must be a signed int literal")),
        }
    }
    fn mutex_int(&mut self) -> R<()> {
        self.require("Mutex")?;
        self.expect(Tok::Lt, "expected Mutex<int>")?;
        self.require("int")?;
        self.expect(Tok::Gt, "expected closing Mutex<int>")?;
        Ok(())
    }
    fn effect(&mut self) -> bool { self.word("susp") || self.word("suspends") }
    fn call(&mut self, method: &str, arg: &str) -> R<()> {
        self.require(method)?;
        self.expect(Tok::LParen, "expected call argument list")?;
        self.require(arg)?;
        self.expect(Tok::RParen, "expected ')' after call")?;
        Ok(())
    }
}

#[derive(Debug)]
struct Increment { name: String, delta: i64 }
fn parse_increment(p: &mut Cursor<'_>) -> R<Increment> {
    let pos = p.pos();
    // Exporting increment would allow an unverified host or second actor
    // to invoke it after main; the lock-elision proof needs a private helper.
    if p.word("pub") { return Err(error(pos,"E_LOCK_UNSUPPORTED","mutex fast-path increment must be private")); }
    if !p.effect() {
        return Err(error(pos,"E_AWAIT_EFFECT","lock_async requires a suspending callable"));
    }
    p.require("fnc")?;
    let function = p.name()?;
    if function == "main" { return Err(error(pos,"E_LOCK_UNSUPPORTED","expected private increment function")); }
    p.expect(Tok::LParen, "expected '(' after function name")?;
    p.mutex_int()?;
    let handle = p.name()?;
    p.expect(Tok::RParen, "expected ')' after mutex loan")?;
    p.expect(Tok::Colon, "expected declared int return")?;
    p.require("int")?;
    p.expect(Tok::LBrace, "expected function body")?;

    p.require("using")?;
    let _explicit_await = p.word("await");
    p.require(&handle)?;
    p.expect(Tok::Dot, "expected lock_async member call")?;
    p.require("lock_async")?;
    p.expect(Tok::LParen, "expected no-argument lock_async")?;
    p.expect(Tok::RParen, "expected no-argument lock_async")?;
    p.require("as")?;
    let lease = p.name()?;
    if lease == handle { return Err(error(p.pos(),"E_LOCK_UNSUPPORTED","lease cannot shadow its owner")); }
    p.expect(Tok::LBrace, "expected lexical lock scope")?;

    p.require(&lease)?;
    p.expect(Tok::Dot, "expected guarded lease.value")?;
    p.require("value")?;
    p.expect(Tok::PlusAssign, "expected checked += under guard")?;
    let delta = p.int()?;
    p.expect(Tok::Semi, "expected ';' after guarded update")?;

    p.require("return")?;
    p.require(&lease)?;
    p.expect(Tok::Dot, "lease escape is prohibited; return a copied scalar value")?;
    p.require("value")?;
    p.expect(Tok::Semi, "expected ';' after return")?;
    p.expect(Tok::RBrace, "expected end of scoped guard")?;
    p.expect(Tok::RBrace, "expected end of increment function")?;
    Ok(Increment { name: function, delta })
}

#[derive(Clone, Copy, Debug)]
enum MainKind { ReturnObserved, PrintAndZero, LazyAndZero }
#[derive(Debug)]
struct Main { initial: i64, kind: MainKind }

fn parse_main(p: &mut Cursor<'_>, increment: &str) -> R<Main> {
    let pos = p.pos();
    p.require("pub")?;
    let is_susp = p.effect();
    p.require("fnc")?;
    p.require("main")?;
    p.expect(Tok::LParen, "expected main()")?;
    p.expect(Tok::RParen, "expected main()")?;
    p.expect(Tok::Colon, "expected int return")?;
    p.require("int")?;
    p.expect(Tok::LBrace, "expected main body")?;
    p.require("val")?;
    let owner = p.name()?;
    p.expect(Tok::Assign, "expected local mutex factory")?;
    p.mutex_int()?;
    p.expect(Tok::LParen, "expected Mutex<int>(value)")?;
    let initial = p.int()?;
    p.expect(Tok::RParen, "expected closing mutex factory")?;
    p.expect(Tok::Semi, "expected ';' after mutex construction")?;

    let kind = if p.word("return") {
        if !is_susp { return Err(error(pos,"E_AWAIT_EFFECT","await requires a suspending main")); }
        p.require("await")?;
        p.call(increment, &owner)?;
        p.expect(Tok::Semi, "expected ';' after result")?;
        MainKind::ReturnObserved
    } else if p.word("stdio") {
        if !is_susp { return Err(error(pos,"E_AWAIT_EFFECT","await requires a suspending main")); }
        p.expect(Tok::Dot, "expected stdio.println")?;
        p.require("println")?;
        p.expect(Tok::LParen, "expected println(argument)")?;
        p.require("await")?;
        p.call(increment, &owner)?;
        p.expect(Tok::RParen, "expected ')' after println")?;
        p.expect(Tok::Semi, "expected ';' after println")?;
        p.require("return")?;
        let zero = p.int()?;
        if zero != 0 { return Err(error(p.pos(),"E_LOCK_UNSUPPORTED","stdout profile main must return 0")); }
        p.expect(Tok::Semi, "expected ';' after return")?;
        MainKind::PrintAndZero
    } else if p.word("val") {
        // Ordinary function call creates an unstarted lazy task. The compiler
        // may DCE this task because both arguments are constant and there is
        // no explicit scheduling/driving effect or observable side effect.
        if is_susp { return Err(error(pos,"E_LOCK_UNSUPPORTED","lazy-only profile requires plain main")); }
        let pending = p.name()?;
        if pending == owner { return Err(error(p.pos(),"E_LOCK_UNSUPPORTED","task and owner names must differ")); }
        p.expect(Tok::Assign, "expected lazy task binding")?;
        p.call(increment, &owner)?;
        p.expect(Tok::Semi, "expected ';' after lazy task")?;
        p.require("return")?;
        let zero = p.int()?;
        if zero != 0 { return Err(error(p.pos(),"E_LOCK_UNSUPPORTED","lazy profile must return 0")); }
        p.expect(Tok::Semi, "expected ';' after return")?;
        MainKind::LazyAndZero
    } else {
        return Err(error(p.pos(),"E_LOCK_UNSUPPORTED","unsupported mutex single-task main body"));
    };
    p.expect(Tok::RBrace, "expected closing main body")?;
    p.expect(Tok::Eof, "extra top-level declarations prevent lock elision")?;
    Ok(Main { initial, kind })
}

fn lit(v: i64) -> TE { TE { ty: Type::Int, e: TExpr::I64(v) } }
fn global(index: u32) -> TE { TE { ty: Type::Int, e: TExpr::GlobalGet(index) } }
fn local(index: u32) -> TE { TE { ty: Type::Int, e: TExpr::Local(index) } }
fn call_increment() -> TE { TE { ty: Type::Int, e: TExpr::Call(0, vec![]) } }

fn lower(inc: Increment, main: Main) -> R<TProgram> {
    // No panic/abort path can occur while holding the single-task guard:
    // reject even a statically-proven overflowing arithmetic operation.
    // (The emitted i64 addition still runs through the checked Wasm helper.)
    if main.initial.checked_add(inc.delta).is_none() {
        return Err(error(Pos { line:1,col:1 },"E_OVERFLOW",
            "constant guarded mutation would overflow; no unwind cleanup on this profile"));
    }
    if matches!(main.kind, MainKind::LazyAndZero) {
        return Ok(TProgram {
            funcs: vec![TFunc { name:"main".into(),params:vec![],ret:Type::Int,
                locals:vec![],body:vec![TStmt::Return(Some(lit(0)))] }],
            generators:vec![],main:0,imports:vec![],data:vec![],heap:false,
            globals:vec![],actors:vec![],strings:false,guest_eh:false,
            raise_tag:false,catch_depth:false,
        });
    }

    let busy = TE { ty:Type::Bool, e:TExpr::Bin(
        BinOp::Ne,Box::new(global(1)),Box::new(lit(0))) };
    let update = TE { ty:Type::Int, e:TExpr::Bin(
        BinOp::Add,Box::new(global(0)),Box::new(lit(inc.delta))) };
    let increment_body = vec![
        // Per-instance runtime lock flag; this is a checked single-task
        // lease, not a shareable or transportable mutex handle.
        TStmt::If(vec![(busy,vec![TStmt::Unreachable])],None),
        TStmt::GlobalSet(1,lit(1)),
        TStmt::GlobalSet(0,update),
        TStmt::Set(0,global(0)),
        TStmt::GlobalSet(1,lit(0)), // cleanup always precedes return
        TStmt::Return(Some(local(0))),
    ];
    let (imports,main_tail) = match main.kind {
        MainKind::ReturnObserved => (vec![],vec![TStmt::Return(Some(call_increment()))]),
        MainKind::PrintAndZero => (
            vec![Import::PrintI64,Import::PrintNewline],
            vec![TStmt::Expr(TE {ty:Type::Void,e:TExpr::Print {
                value:Box::new(call_increment()),newline:true,
            }}),TStmt::Return(Some(lit(0)))],
        ),
        MainKind::LazyAndZero => unreachable!(),
    };
    let mut main_body = vec![TStmt::GlobalSet(0,lit(main.initial)),TStmt::GlobalSet(1,lit(0))];
    main_body.extend(main_tail);
    Ok(TProgram {
        funcs: vec![
            TFunc {name:inc.name,params:vec![],ret:Type::Int,
                locals:vec![Type::Int],body:increment_body},
            TFunc {name:"main".into(),params:vec![],ret:Type::Int,
                locals:vec![],body:main_body},
        ],
        generators:vec![],main:1,imports,data:vec![],heap:false,
        globals:vec![Type::Int,Type::Int],actors:vec![],strings:false,
        guest_eh:false,raise_tag:false,catch_depth:false,
    })
}

fn guarded_suspension(tokens: &[Token]) -> Option<Pos> {
    for (i, token) in tokens.iter().enumerate() {
        if !matches!(&token.tok, Tok::Ident(name) if name == "using") { continue; }
        let Some(open) = tokens.iter().enumerate().skip(i).find_map(|(j,t)|
            (t.tok == Tok::LBrace).then_some(j)) else { continue };
        let mut depth = 1usize;
        for tok in tokens.iter().skip(open+1) {
            match &tok.tok {
                Tok::LBrace => depth += 1,
                Tok::RBrace => { depth -= 1; if depth == 0 { break; } },
                Tok::Ident(name) if matches!(name.as_str(),"await"|"select"|"readch"|"yield") =>
                    return Some(tok.pos),
                _ => {},
            }
        }
    }
    None
}

/// A narrow scanner selects only source using the reserved intrinsic and
/// scoped lock_async spelling. It never claims general Mutex<T> support.
pub fn try_lower(tokens: &[Token]) -> R<Option<TProgram>> {
    let has_mutex = tokens.windows(2).any(|w| matches!(
        (&w[0].tok,&w[1].tok),(Tok::Ident(name),Tok::Lt) if name=="Mutex"
    ));
    let lock_async = tokens.windows(3).any(|w| matches!(
        (&w[0].tok,&w[1].tok,&w[2].tok),
        (Tok::Dot,Tok::Ident(name),Tok::LParen) if name=="lock_async"
    ));
    if !has_mutex || !lock_async { return Ok(None); }
    if let Some(pos) = guarded_suspension(tokens) {
        return Err(error(pos,"E_GUARD_SUSPEND","await/select/readch/yield inside live MutexLease"));
    }
    // This restricted parser owns *all* candidate source; never fall back to
    // the generic language parser and silently erase guard/await effects.
    let mut p = Cursor::new(tokens);
    let increment = parse_increment(&mut p)?;
    let main = parse_main(&mut p,&increment.name)?;
    lower(increment,main).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    const SOURCE: &str = r#"
susp fnc increment(Mutex<int> counter): int {
  using await counter.lock_async() as lease {
    lease.value += 1;
    return lease.value;
  }
}
pub susp fnc main(): int {
  val counter = Mutex<int>(0);
  return await increment(counter);
}
"#;
    fn parse(source: &str) -> R<Option<TProgram>> {
        try_lower(&crate::lexer::lex(source).unwrap())
    }
    #[test]
    fn guarded_scalar_lowering_keeps_runtime_state_and_cleanup() {
        let program = parse(SOURCE).unwrap().unwrap();
        assert_eq!(program.globals,vec![Type::Int,Type::Int]);
        assert_eq!(program.funcs.len(),2);
        assert_eq!(program.main,1);
        assert!(matches!(program.funcs[0].body[4],TStmt::GlobalSet(1,_)));
        assert!(matches!(program.funcs[0].body[5],TStmt::Return(_)));
        let wasm = crate::compile(SOURCE).unwrap();
        assert_eq!(&wasm[..8], b"\0asm\x01\0\0\0");
        assert_eq!(wasm,crate::compile(SOURCE).unwrap());
    }
    #[test]
    fn implicit_and_explicit_acquisition_are_semantically_identical() {
        let other = SOURCE.replace("using await counter.lock_async()", "using counter.lock_async()");
        assert_eq!(crate::compile(SOURCE).unwrap(),crate::compile(&other).unwrap());
    }
    #[test]
    fn no_spoofed_scope_and_no_extra_top_level() {
        let extra = format!("{} pub fnc other(): int {{return 1;}}",SOURCE);
        assert_eq!(parse(&extra).unwrap_err().code,"E_LOCK_UNSUPPORTED");
        let invalid = SOURCE.replace("return lease.value;", "return lease;");
        assert_eq!(parse(&invalid).unwrap_err().code,"E_LOCK_UNSUPPORTED");
    }
    #[test]
    fn ready_await_inside_lease_is_a_compile_error() {
        let bad = SOURCE.replace("lease.value += 1;", "lease.value += await 42;");
        assert_eq!(parse(&bad).unwrap_err().code,"E_GUARD_SUSPEND");
    }
    #[test]
    fn constant_overflow_rejects_before_emission() {
        let bad = SOURCE.replace("Mutex<int>(0)", "Mutex<int>(9223372036854775807)");
        assert_eq!(parse(&bad).unwrap_err().code,"E_OVERFLOW");
    }
}
