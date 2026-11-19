//! Native host for the same compiler core: `attoc input.atto [-o out.wasm]`.
//! Used for native-versus-browser differential checks.
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (input, output, check_only, ownership) = match args.as_slice() {
        [flag] if flag == "--help" || flag == "-h" => {
            println!("usage: attoc [--borrow-v1] <input.atto> [-o out.wasm]\n       attoc [--borrow-v1] --check <input.atto>");
            return ExitCode::SUCCESS;
        }
        [flag, i] if flag == "--check" =>
            (i.clone(), None, true, atto_compiler::OwnershipMode::LegacyMoves),
        [mode, flag, i] if mode == "--borrow-v1" && flag == "--check" =>
            (i.clone(), None, true, atto_compiler::OwnershipMode::BorrowV1),
        [i] =>
            (i.clone(), None, false, atto_compiler::OwnershipMode::LegacyMoves),
        [i, o, out] if o == "-o" =>
            (i.clone(), Some(out.clone()), false, atto_compiler::OwnershipMode::LegacyMoves),
        [mode, i] if mode == "--borrow-v1" =>
            (i.clone(), None, false, atto_compiler::OwnershipMode::BorrowV1),
        [mode, i, o, out] if mode == "--borrow-v1" && o == "-o" =>
            (i.clone(), Some(out.clone()), false, atto_compiler::OwnershipMode::BorrowV1),
        _ => {
            eprintln!("usage: attoc [--borrow-v1] <input.atto> [-o out.wasm]\n       attoc [--borrow-v1] --check <input.atto>");
            return ExitCode::from(2);
        }
    };
    let source = match std::fs::read(&input).map(String::from_utf8) {
        Ok(Ok(s)) => s,
        _ => {
            eprintln!("{input}: cannot read UTF-8 source");
            return ExitCode::from(2);
        }
    };
    match atto_compiler::compile_with_mode(&source, ownership) {
        Ok(bytes) => {
            if check_only {
                return ExitCode::SUCCESS;
            }
            if let Some(o) = output {
                if let Err(err) = std::fs::write(&o, &bytes) {
                    eprintln!("{o}: cannot write: {err}");
                    return ExitCode::from(2);
                }
            } else {
                use std::io::Write;
                if let Err(err) = std::io::stdout().write_all(&bytes) {
                    eprintln!("stdout: cannot write compiled Wasm: {err}");
                    return ExitCode::from(2);
                }
            }
            ExitCode::SUCCESS
        }
        Err(d) => {
            eprintln!("{input}:{d}");
            ExitCode::from(1)
        }
    }
}
