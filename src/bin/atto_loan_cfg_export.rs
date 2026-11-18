//! Host-only diagnostic tool. Emits portable, versioned compiler-checked
//! typed CFG loan facts to stdout for the independent canonical Rust checker.
//! The emitted text is NOT an attested source/Wasm operation proof.
use std::{fs, process::ExitCode};

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let [source_file] = args.as_slice() else {
        eprintln!("usage: atto-loan-cfg-export <source.atto>");
        return ExitCode::from(2);
    };
    let source = match fs::read_to_string(source_file) {
        Ok(s) => s,
        Err(error) => {
            eprintln!("cannot read {source_file}: {error}");
            return ExitCode::from(2);
        }
    };
    let checked = (|| {
        // No diagnostic may be exported for source rejected by the normal
        // production compiler. Recheck the exact input, still without
        // claiming independent emitter completeness.
        atto_compiler::compile(&source)?;
        let tokens = atto_compiler::lexer::lex(&source)?;
        let ast = atto_compiler::parser::Parser::new(tokens).parse_program()?;
        let typed = atto_compiler::check::check(&ast)?;
        atto_compiler::check::loan_cfg_wire_v1::export_checked_program(&typed)
    })();
    match checked {
        Ok(manifest) => {
            print!("{manifest}");
            ExitCode::SUCCESS
        }
        Err(diagnostic) => {
            eprintln!("{source_file}: {diagnostic}");
            ExitCode::from(1)
        }
    }
}
