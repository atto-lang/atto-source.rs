//! AttoLang browser compiler: `.atto` source -> WebAssembly, itself built for
//! `wasm32-unknown-unknown` so the playground compiles entirely client-side.
//!
//! Syntax follows `docs/grammar.ebnf` of the Java reference compiler; `int`
//! follows the LLVM backend and spec chapter 17 (signed 64-bit, checked).
//! This is a subset compiler: every construct outside the admitted subset is
//! rejected with a diagnostic and never produces an artifact. See README.md.

pub mod ast;
pub mod check;
pub mod diag;
pub mod lexer;
pub mod parser;
pub mod wasm;

pub use diag::{Diagnostic, Pos};

pub const SOURCE_CAP: usize = 64 * 1024;
pub const ABI_VERSION: u32 = 2;
pub const COMPILER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Compile one `.atto` source file to a guest WebAssembly module.
pub fn compile(source: &str) -> Result<Vec<u8>, Diagnostic> {
    if source.len() > SOURCE_CAP {
        return Err(Diagnostic::new(
            Pos { line: 1, col: 1 },
            "E_SOURCE_LIMIT",
            format!("source exceeds {SOURCE_CAP} UTF-8 bytes"),
        ));
    }
    let toks = lexer::lex(source)?;
    let program = parser::Parser::new(toks).parse_program()?;
    if !program.actor_handlers.is_empty() {
        return Err(Diagnostic::new(
            Pos { line: 1, col: 1 },
            "E_UNSUPPORTED",
            "actor receive declarations require the separate scalar actor compiler ABI",
        ));
    }
    let typed = check::check(&program)?;
    let bytes = wasm::emit(&typed);
    if bytes.len() > wasm::MAX_OUTPUT {
        return Err(Diagnostic::new(
            Pos { line: 1, col: 1 },
            "E_OUTPUT_LIMIT",
            "generated module exceeds 1 MiB",
        ));
    }
    Ok(bytes)
}

/// Compile a single scalar actor `receive(int)->int` handler into the
/// import-free browser worker contract `atto.browser.actor.scalar.v0`.
/// This is a stateless handler prototype: not spawn, ownership or supervision.
pub fn compile_actor(source: &str) -> Result<Vec<u8>, Diagnostic> {
    use ast::{Expr, ExprKind, Func, Stmt, Type};
    if source.len() > SOURCE_CAP {
        return Err(Diagnostic::new(Pos { line: 1, col: 1 }, "E_SOURCE_LIMIT", "actor source exceeds 64 KiB"));
    }
    let tokens = lexer::lex(source)?;
    let mut program = parser::Parser::new(tokens).parse_program()?;
    if program.actor_handlers.len() != 1 {
        return Err(Diagnostic::new(
            Pos { line: 1, col: 1 }, "E_ACTOR_ENTRY", "exactly one scalar actor receive handler is required",
        ));
    }
    let handler = program.actor_handlers[0].clone();
    if program.funcs.iter().any(|f| f.name == "main") {
        return Err(Diagnostic::new(Pos { line: 1, col: 1 }, "E_ACTOR_ENTRY", "actor module cannot define main"));
    }
    let pos = Pos { line: 1, col: 1 };
    // Only for reusing the already validated, checked scalar IR pipeline.
    // Not exported to the actor worker and not present in user source.
    program.funcs.push(Func {
        name: "main".into(), pos, params: Vec::new(), ret: Type::Int,
        body: vec![Stmt::Return(Some(Expr { kind: ExprKind::Int(Some(0)), pos }), pos)],
    });
    let typed = check::check(&program)?;
    if !typed.imports.is_empty() || !typed.data.is_empty() {
        return Err(Diagnostic::new(pos, "E_ACTOR_IMPORT", "actor guest cannot use host imports or linear memory"));
    }
    let index = typed.funcs.iter().position(|f| f.name == handler).expect("checked handler retained");
    let bytes = wasm::emit_actor(&typed, index as u32);
    if bytes.len() > wasm::MAX_OUTPUT {
        return Err(Diagnostic::new(pos, "E_OUTPUT_LIMIT", "actor module exceeds 1 MiB"));
    }
    Ok(bytes)
}

/// Browser host ABI (`atto-playground` compiler contract v2). One instance is
/// owned by one worker; callers copy results out before compiling again.
#[cfg(target_arch = "wasm32")]
mod abi {
    use super::*;
    use std::cell::RefCell;

    const ERROR_CAP: usize = 512;

    struct State {
        bytes: Vec<u8>,
        error: [u8; ERROR_CAP],
        line: u32,
        col: u32,
    }

    thread_local! {
        static STATE: RefCell<State> = const { RefCell::new(State { bytes: Vec::new(), error: [0; ERROR_CAP], line: 0, col: 0 }) };
    }
    static mut INPUT: [u8; SOURCE_CAP] = [0; SOURCE_CAP];

    #[no_mangle]
    pub extern "C" fn abi_version() -> u32 {
        ABI_VERSION
    }
    #[no_mangle]
    pub extern "C" fn input_ptr() -> u32 {
        std::ptr::addr_of_mut!(INPUT) as *mut u8 as usize as u32
    }
    #[no_mangle]
    pub extern "C" fn input_capacity() -> u32 {
        SOURCE_CAP as u32
    }
    #[no_mangle]
    pub extern "C" fn result_ptr() -> u32 {
        STATE.with(|s| s.borrow().bytes.as_ptr() as usize as u32)
    }
    #[no_mangle]
    pub extern "C" fn result_length() -> u32 {
        STATE.with(|s| s.borrow().bytes.len() as u32)
    }
    #[no_mangle]
    pub extern "C" fn error_ptr() -> u32 {
        STATE.with(|s| s.borrow().error.as_ptr() as usize as u32)
    }
    #[no_mangle]
    pub extern "C" fn error_line_number() -> u32 {
        STATE.with(|s| s.borrow().line)
    }
    #[no_mangle]
    pub extern "C" fn error_column_number() -> u32 {
        STATE.with(|s| s.borrow().col)
    }

    /// Returns 0 on success, 1 on a diagnostic. On error `result_length` is 0.
    #[no_mangle]
    pub extern "C" fn compile(length: i32) -> i32 {
        compile_with(length, false)
    }
    /// ABI v2-compatible additional export, returns the same result/error
    /// buffer but compiles a distinct import-free actor guest contract.
    #[no_mangle]
    pub extern "C" fn compile_actor(length: i32) -> i32 {
        compile_with(length, true)
    }
    fn compile_with(length: i32, actor: bool) -> i32 {
        STATE.with(|s| {
            let mut st = s.borrow_mut();
            st.bytes = Vec::new();
            st.error = [0; ERROR_CAP];
            st.line = 0;
            st.col = 0;
            let fail = |st: &mut State, d: Diagnostic| {
                let msg = d.render();
                let mut n = msg.len().min(ERROR_CAP - 1);
                while !msg.is_char_boundary(n) {
                    n -= 1;
                }
                st.error[..n].copy_from_slice(&msg.as_bytes()[..n]);
                st.line = d.pos.line;
                st.col = d.pos.col;
                1
            };
            if length < 0 || length as usize > SOURCE_CAP {
                return fail(
                    &mut st,
                    Diagnostic::new(
                        Pos { line: 1, col: 1 },
                        "E_SOURCE_LIMIT",
                        format!("source exceeds {SOURCE_CAP} UTF-8 bytes"),
                    ),
                );
            }
            // SAFETY: single-threaded wasm; the host wrote `length` bytes before calling.
            let input: &[u8] = unsafe { &(*std::ptr::addr_of!(INPUT))[..length as usize] };
            let source = match std::str::from_utf8(input) {
                Ok(s) => s,
                Err(_) => {
                    return fail(
                        &mut st,
                        Diagnostic::new(Pos { line: 1, col: 1 }, "E_UTF8", "source is not valid UTF-8"),
                    )
                }
            };
            match if actor { super::compile_actor(source) } else { super::compile(source) } {
                Ok(bytes) => {
                    st.bytes = bytes;
                    0
                }
                Err(d) => fail(&mut st, d),
            }
        })
    }
}
