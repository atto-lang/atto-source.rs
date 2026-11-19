//! Native host for the same compiler core: `attoc input.atto [-o out.wasm]`.
//! Used for native-versus-browser differential checks.
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Opt-in source compilation mode: legacy callers remain unchanged.
    // In borrow-v1, plain top-level Foo formals lower to the same checked
    // readonly Borrow<Foo> call ABI, with unsafe forms rejected.
    let (input, output, ownership) = match args.as_slice() {
        [i] => (i.clone(), None, atto_compiler::OwnershipMode::LegacyMoves),
        [i, o, out] if o == "-o" =>
            (i.clone(), Some(out.clone()), atto_compiler::OwnershipMode::LegacyMoves),
        [mode, i] if mode == "--borrow-v1" =>
            (i.clone(), None, atto_compiler::OwnershipMode::BorrowV1),
        [mode, i, o, out] if mode == "--borrow-v1" && o == "-o" =>
            (i.clone(), Some(out.clone()), atto_compiler::OwnershipMode::BorrowV1),
        _ => {
            eprintln!("usage: attoc [--borrow-v1] <input.atto> [-o out.wasm]");
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
            if let Some(o) = output {
                if std::fs::write(&o, &bytes).is_err() {
                    eprintln!("{o}: cannot write");
                    return ExitCode::from(2);
                }
            } else {
                use std::io::Write;
                std::io::stdout().write_all(&bytes).ok();
            }
            ExitCode::SUCCESS
        }
        Err(d) => {
            eprintln!("{input}:{d}");
            ExitCode::from(1)
        }
    }
}
