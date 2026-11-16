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
pub mod generator_cfg;
pub mod generator_frame;
pub mod lexer;
pub mod io_shorthand;
pub mod detached_io;
pub mod parser;
/// Bounded reference scheduler gate; not yet wired to guest Wasm execution.
pub mod state_admission;
pub mod target_syntax;
pub mod target_typed;
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
    // Detached tasks require ownership, completion callbacks and supervisor
    // error routing. Parse-check early; NEVER execute them as synchronous I/O.
    if let Some(detached) = detached_io::inspect(&toks)?.first() {
        return Err(Diagnostic::new(detached.pos, "E_UNSUPPORTED",
            "supervised do readch/writech syntax recognized, but detached I/O scheduling and completion callbacks are not implemented"));
    }
    // 'pipe' is continuous transport. It never reuses one-shot '>>'
    // or '<<' and must not manufacture a channel/actor graph without
    // typed capabilities, bounded backpressure and supervisor ownership.
    if let Some(pipe) = io_shorthand::inspect_pipe(&toks)?.first() {
        return Err(Diagnostic::new(pipe.pos, "E_UNSUPPORTED",
            "continuous pipe requires a typed channel/stream/actor transport runtime; syntax is recognized but the browser guest ABI is not implemented"));
    }
    // Keep strict syntax and effect diagnostics. Valid unary reads continue
    // into the production AST/checker, which grants only typed Channel<int>
    // reads and emits the versioned Worker-owned JSPI import contract.
    let unary_reads = io_shorthand::inspect_unary_reads(&toks)?;
    io_shorthand::validate_unary_read_suspension(&toks, &unary_reads)?;
    // The browser guest already has a typed stdio.print output import.
    // Permit the exact stdio.out << value statement form to lower through
    // that existing checked path. All other channels, streams, read
    // directions and select registrations fail closed until implemented.
    for io in io_shorthand::inspect(&toks)? {
        if io.source_or_sink == "stdio.out"
            && io.direction == io_shorthand::Direction::Write
            && io.context == io_shorthand::Context::Standalone
            && io.mode == io_shorthand::Mode::Direct
            && !io.read_binder_on_left
        {
            continue;
        }
        // A direct declaration-first capture has precisely the same checked
        // AST/type/effect path as unary `<< c`. Permit it to reach the actual
        // production parser and checker; never grant select or nb/try forms.
        if io.direction == io_shorthand::Direction::Read
            && io.context == io_shorthand::Context::Standalone
            && io.mode == io_shorthand::Mode::Direct
            && io.read_binder_on_left
        {
            continue;
        }
        return Err(Diagnostic::new(io.pos, "E_UNSUPPORTED",
            "I/O direction shorthand recognized, but this capability is not supported by the browser compiler; only standalone stdio.out writes are executable"));
    }
    // Detect target declarations at any file-scope position, not only at
    // source byte zero. Ignore comments/strings (the lexer already tokenizes
    // them) and ignore nested brace bodies so a string or local expression
    // cannot accidentally switch the compiler into the target grammar.
    let mut braces = 0usize;
    let mut ordinary_class_scope = 0usize;
    let mut target = false;
    // Top-level `mtd` selects the receiver declaration grammar, whereas a
    // member `mtd` inside `define class ... as ... end` belongs to the
    // canonical class parser even if the class uses an end-delimited body.
    // A brace-only depth check wrongly classified that member as top-level.
    for i in 0..toks.len() {
        match &toks[i].tok {
            lexer::Tok::LBrace => braces = braces.saturating_add(1),
            lexer::Tok::RBrace => braces = braces.saturating_sub(1),
            lexer::Tok::Ident(word) if braces == 0 => {
                let next = toks.get(i + 1).map(|token| &token.tok);
                if word == "define" &&
                    matches!(next, Some(lexer::Tok::Ident(name)) if name == "class") {
                    ordinary_class_scope = ordinary_class_scope.saturating_add(1);
                    continue;
                }
                if word == "end" && ordinary_class_scope > 0 {
                    ordinary_class_scope -= 1;
                    continue;
                }
                if ordinary_class_scope > 0 {
                    continue;
                }
                if word == "declare" || word == "mtd" ||
                    (word == "use" && matches!(next,
                        Some(lexer::Tok::Ident(name)) if name == "namespace")) {
                    target = true;
                    break;
                }
            }
            _ => {}
        }
    }
    let program = if target {
        let headers = target_syntax::parse_target_declarations(&toks)?;
        if !headers.classes.is_empty() {
            // Out-of-line *immutable class* mtd is executable: translate the
            // receiver headers into equivalent in-class method tokens, then
            // reuse the full production parser, checker and guest Wasm emitter.
            // Namespace, imports, actor receivers and mutable-self are gated
            // inside the lowering helper, never silently compiled as classes.
            let lowered = target_syntax::lower_class_receivers(&toks, &headers)?;
            parser::Parser::new(lowered).parse_program()?
        } else {
            // Actor mtd stays strictly syntax/type preflight only. An ActorRef
            // must never acquire unverified synchronous method dispatch.
            let typed = target_typed::lower_target_signatures(&headers)?;
            let actor_ast = target_typed::build_actor_program(&headers, &typed);
            check::check_target_actor_interface(&actor_ast)?;
            return Err(Diagnostic::new(toks[0].pos, "E_UNSUPPORTED",
                "namespace-scoped actor declarations and mtd methods are syntax-only, not browser-lowered"));
        }
    } else {
        parser::Parser::new(toks).parse_program()?
    };
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
            let input: &[u8] = unsafe {
                std::slice::from_raw_parts(std::ptr::addr_of!(INPUT).cast::<u8>(), length as usize)
            };
            let source = match std::str::from_utf8(input) {
                Ok(s) => s,
                Err(_) => {
                    return fail(
                        &mut st,
                        Diagnostic::new(Pos { line: 1, col: 1 }, "E_UTF8", "source is not valid UTF-8"),
                    )
                }
            };
            match super::compile(source) {
                Ok(bytes) => {
                    st.bytes = bytes;
                    0
                }
                Err(d) => fail(&mut st, d),
            }
        })
    }
}
