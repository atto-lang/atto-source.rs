//! AttoLang browser compiler: `.atto` source -> WebAssembly, itself built for
//! `wasm32-unknown-unknown` so the playground compiles entirely client-side.
//!
//! Syntax follows `docs/grammar.ebnf` of the Java reference compiler; `int`
//! follows the LLVM backend and spec chapter 17 (signed 64-bit, checked).
//! This is a subset compiler: every construct outside the admitted subset is
//! rejected with a diagnostic and never produces an artifact. See README.md.

pub mod ast;
pub mod check;
/// Host/runtime checked-handle foundation; NOT yet a guest `ref!` ABI.
pub mod checked_handle_runtime;
pub mod diag;
pub mod generator_cfg;
pub mod generator_frame;
pub mod lexer;
pub mod io_shorthand;
pub mod detached_io;
pub mod parser;
/// Checked single-source module functions; external imports remain unlinked.
pub mod module_lower;
/// Fail-closed monomorphic scalar Any specialization; no dynamic Any ABI.
pub mod any_type_lower;
/// Source-local scalar products use the checked class/constructor guest ABI.
pub mod shape_lower;
/// Checked source-local trait and class composition.
pub mod nominal_lower;
/// Explicitly unlinked foreign prototypes and source-only exports.
pub mod visibility_lower;
/// Direct routine-only and statically specialized named callbacks.
pub mod routine_lower;
/// Bounded reference scheduler gate; not yet wired to guest Wasm execution.
pub mod state_admission;
pub mod target_syntax;
pub mod target_typed;
pub mod wasm;

#[cfg(test)]
mod borrow_mut_tests;

pub use diag::{Diagnostic, Pos};

pub const SOURCE_CAP: usize = 64 * 1024;
pub const ABI_VERSION: u32 = 2;
pub const COMPILER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Migration gate for the language decision that bare *class* formals
/// borrow by default. Existing packages keep the historical consuming ABI
/// until they opt in; this avoids changing the meaning of source silently.
/// The source marker is intentionally versioned and constrained to the first
/// line. Package/CLI profile discovery can replace this migration mechanism
/// once it is ratified without changing the checked BorrowClass ABI.
fn prepare_borrow_default_v1(program: &mut ast::Program) -> Result<(), Diagnostic> {
    use ast::{Type, ParamOwnership};
    use std::collections::BTreeSet;
    let classes: BTreeSet<String> = program.classes.iter().map(|c| c.name.clone()).collect();
    let type_names = &program.type_names;

    // Constructor, class-method and actor-method class loans require method
    // receiver effects and inter-domain provenance not yet implemented in
    // this compiler. Do not let v1 silently retain their legacy owned ABI.
    for class in &program.classes {
        for params in class.methods.iter().map(|m| m.params.as_slice())
            .chain(class.ctor.iter().map(|c| c.params.as_slice()))
        {
            for p in params {
                if let Type::Named(i) = p.ty {
                    if classes.contains(&type_names[i as usize].0) && !p.explicit_take {
                        return Err(Diagnostic::new(p.pos, "E_UNSUPPORTED_TYPE",
                            "borrow-default-v1 method/constructor class formals need verified receiver and lifetime lowering; use top-level fnc for now"));
                    }
                }
            }
        }
    }
    for actor in &program.actors {
        for method in &actor.methods {
            for p in &method.params {
                if let Type::Named(i) = p.ty {
                    if classes.contains(&type_names[i as usize].0) && !p.explicit_take {
                        return Err(Diagnostic::new(p.pos, "E_SHARE_DOMAIN",
                            "borrow-default-v1 actor method class loans require explicit domain admission"));
                    }
                }
            }
        }
    }
    for function in &mut program.funcs {
        for p in &mut function.params {
            if let Type::Named(i) = p.ty {
                if classes.contains(&type_names[i as usize].0) {
                    if p.explicit_take {
                        p.ownership = ParamOwnership::Take;
                    } else if p.mutable {
                        // Unlike a mutable owned argument, this is a
                        // caller-proven exclusive loan over the same class
                        // allocation. No ownership is moved or cloned.
                        p.ty = Type::BorrowMutNamed(i);
                        p.ownership = ParamOwnership::BorrowWrite;
                    } else {
                        p.ty = Type::BorrowNamed(i);
                        p.ownership = ParamOwnership::BorrowRead;
                    }
                }
            }
        }
    }
    program.borrow_default_v1 = true;
    Ok(())
}

/// Ownership contract selected by the *invoking* compiler/package, not
/// inferred from a callee's implementation. Imported signatures will require
/// compatible versioned metadata before cross-package linking is supported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnershipProfile {
    Legacy,
    BorrowDefaultV1,
}

/// Compile using a source's first-line migration marker when present.
/// Existing browser ABI and unversioned source behavior remain unchanged.
pub fn compile(source: &str) -> Result<Vec<u8>, Diagnostic> {
    compile_inner(source, None)
}

/// An explicit compiler/package integration point. A build driver may select
/// the version without injecting source text; conflicting inline markers
/// reject instead of silently overriding ownership semantics.
pub fn compile_with_profile(
    source: &str, profile: OwnershipProfile
) -> Result<Vec<u8>, Diagnostic> {
    compile_inner(source, Some(profile))
}

/// Compile one `.atto` source file to a guest WebAssembly module.
fn compile_inner(source: &str, selected: Option<OwnershipProfile>) -> Result<Vec<u8>, Diagnostic> {
    if source.len() > SOURCE_CAP {
        return Err(Diagnostic::new(
            Pos { line: 1, col: 1 },
            "E_SOURCE_LIMIT",
            format!("source exceeds {SOURCE_CAP} UTF-8 bytes"),
        ));
    }
    let mut visibility = visibility_lower::lower(lexer::lex(source)?)?;
    let toks = any_type_lower::lower(routine_lower::lower(shape_lower::lower(nominal_lower::lower(module_lower::lower(visibility.tokens)?)?)?)?)?;
    // Detached tasks require ownership, completion callbacks and supervisor
    // error routing. Parse-check early; NEVER execute them as synchronous I/O.
    if let Some(detached) = detached_io::inspect(&toks)?.first() {
        return Err(Diagnostic::new(detached.pos, "E_UNSUPPORTED",
            "supervised do readch/writech syntax recognized, but detached I/O scheduling and completion callbacks are not implemented"));
    }
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
        return Err(Diagnostic::new(io.pos, "E_UNSUPPORTED",
            "I/O direction shorthand recognized, but this capability is not supported by the browser compiler; only standalone stdio.out writes are executable"));
    }
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
    // Versioned opt-in until the package-level compatibility migration is
    // approved. Unversioned source keeps legacy ownership so existing code
    // cannot silently stop consuming class parameters.
    // Misplaced version headers must reject rather than silently using the
    // *legacy consuming* ABI. An innocuous comment above the marker, or a
    // copy/pasted header in the wrong position, cannot change ownership
    // meaning without an explicit diagnostic. Support an ordinary UTF-8
    // BOM on the very first line only.
    let first_line = source.lines().next().unwrap_or("")
        .trim_start_matches('\u{feff}').trim_end_matches('\r');
    let header = match first_line {
        "// atto-ownership: borrow-default-v1" => Some(OwnershipProfile::BorrowDefaultV1),
        other if other.starts_with("// atto-ownership:") =>
            return Err(Diagnostic::new(Pos { line: 1, col: 1 }, "E_PROFILE",
                "unknown AttoLang ownership profile; use borrow-default-v1")),
        _ => None,
    };
    if let (Some(explicit), Some(header_profile)) = (selected, header) {
        if explicit != header_profile {
            return Err(Diagnostic::new(Pos { line: 1, col: 1 }, "E_PROFILE",
                "explicit compiler ownership profile conflicts with source ownership marker"));
        }
    }
    let borrow_default_v1 = selected.or(header).unwrap_or(OwnershipProfile::Legacy)
        == OwnershipProfile::BorrowDefaultV1;
    for (i, line) in source.lines().enumerate().skip(1) {
        if line.trim_start().starts_with("// atto-ownership:") {
            return Err(Diagnostic::new(Pos { line: i as u32 + 1, col: 1 },
                "E_PROFILE", "ownership migration marker must be on the first source line"));
        }
    }
    let mut program = if target {
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
    if borrow_default_v1 {
        prepare_borrow_default_v1(&mut program)?;
    } else {
        // The new canonical 'mut Foo p' spelling is deliberately unavailable
        // in legacy modules. Do not silently interpret it as an owned mutable
        // parameter when migration metadata was omitted.
        for param in program.funcs.iter().flat_map(|f| f.params.iter()) {
            if param.ownership == ast::ParamOwnership::BorrowWrite {
                return Err(Diagnostic::new(param.pos, "E_PROFILE",
                    "mut Foo borrowed formals require borrow-default-v1"));
            }
            if param.ownership == ast::ParamOwnership::Take {
                return Err(Diagnostic::new(param.pos, "E_PROFILE",
                    "take! Foo consuming formals require borrow-default-v1; legacy owned Foo remains the unversioned form"));
            }
        }
    }
    // Validate exports against the parsed top-level symbol table, never a
    // lexical occurrence inside a class, nested method or foreign prototype.
    visibility_lower::resolve_exports(&mut visibility.exports, &program)?;
    let typed = check::check(&program)?;
    let mut bytes = wasm::emit(&typed);
    visibility_lower::metadata(&mut bytes, &visibility.exports);
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
