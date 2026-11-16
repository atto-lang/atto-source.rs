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

/// Versioned callable-body grammar. This is a *draft* selector; the stable
/// package-manifest/default-version policy is tracked separately.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LanguageVersion {
    /// Historical optional-`::` comma body, with implicit tail return.
    V1,
    /// Explicit `::` comma body; unmarked `end` requires explicit return.
    V2,
}

impl LanguageVersion {
    pub const fn from_number(value: u32) -> Option<Self> {
        match value {
            1 => Some(Self::V1),
            2 => Some(Self::V2),
            _ => None,
        }
    }
}

/// Legacy browser/compiler entry point. The existing draft strict behavior
/// remains unchanged until a coordinated, versioned default migration lands.
pub fn compile(source: &str) -> Result<Vec<u8>, Diagnostic> {
    compile_with_language_version(source, LanguageVersion::V2)
}

/// Compile with an explicit language-version selection. Both paths share the
/// same checker, ownership/effect verification and deterministic Wasm emitter.
pub fn compile_with_language_version(
    source: &str, language_version: LanguageVersion,
) -> Result<Vec<u8>, Diagnostic> {
    if source.len() > SOURCE_CAP {
        return Err(Diagnostic::new(
            Pos { line: 1, col: 1 },
            "E_SOURCE_LIMIT",
            format!("source exceeds {SOURCE_CAP} UTF-8 bytes"),
        ));
    }
    let toks = lexer::lex(source)?;
    // Detect target declarations at any file-scope position, not only at
    // source byte zero. Ignore comments/strings (the lexer already tokenizes
    // them) and ignore nested brace bodies so a string or local expression
    // cannot accidentally switch the compiler into the target grammar.
    let mut braces = 0usize;
    let mut target = false;
    for i in 0..toks.len() {
        match &toks[i].tok {
            lexer::Tok::LBrace => braces = braces.saturating_add(1),
            lexer::Tok::RBrace => braces = braces.saturating_sub(1),
            lexer::Tok::Ident(word) if braces == 0 => {
                let next = toks.get(i + 1).map(|token| &token.tok);
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
        let headers = target_syntax::parse_target_declarations_with_language_version(&toks, language_version)?;
        if !headers.classes.is_empty() {
            // Out-of-line *immutable class* mtd is executable: translate the
            // receiver headers into equivalent in-class method tokens, then
            // reuse the full production parser, checker and guest Wasm emitter.
            // Namespace, imports, actor receivers and mutable-self are gated
            // inside the lowering helper, never silently compiled as classes.
            let lowered = target_syntax::lower_class_receivers(&toks, &headers)?;
            parser::Parser::with_language_version(lowered, language_version).parse_program()?
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
        parser::Parser::with_language_version(toks, language_version).parse_program()?
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

    /// Legacy ABI entry point (draft v2 behavior). No global version state.
    #[no_mangle]
    pub extern "C" fn compile(length: i32) -> i32 {
        compile_versioned(length, 2)
    }

    /// Versioned ABI entry point: 1 = historical comma tails; 2 = strict ::.
    /// A rejected version returns E_LANGUAGE_VERSION with no guest artifact.
    #[no_mangle]
    pub extern "C" fn compile_versioned(length: i32, version: u32) -> i32 {
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
            let language_version = match LanguageVersion::from_number(version) {
                Some(value) => value,
                None => return fail(
                    &mut st,
                    Diagnostic::new(Pos { line: 1, col: 1 }, "E_LANGUAGE_VERSION",
                        "unsupported language version (expected 1 or 2)"),
                ),
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
            match super::compile_with_language_version(source, language_version) {
                Ok(bytes) => {
                    st.bytes = bytes;
                    0
                }
                Err(d) => fail(&mut st, d),
            }
        })
    }
}
