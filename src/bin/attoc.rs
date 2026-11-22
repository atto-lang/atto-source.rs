//! Native host for the same compiler core: `attoc input.atto [-o out.wasm]`.
//! Used for native-versus-browser differential checks.
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (portable_version, rest) = match args.first().map(String::as_str) {
        Some("--portable-v4") => (4,&args[1..]),
        Some("--portable-v5") => (5,&args[1..]),
        Some("--future-int-aot") => (6,&args[1..]),
        _ => (0,&args[..]),
    };
    let (input, output) = match rest {
        [i] => (i.clone(), None),
        [i, o, out] if o == "-o" => (i.clone(), Some(out.clone())),
        _ => {
            eprintln!("usage: attoc [--portable-v4|--portable-v5|--future-int-aot] <input.atto> [-o out.wasm]");
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
    let result=match portable_version {
        4 => atto_compiler::compile_portable_v4(&source),
        5 => atto_compiler::compile_portable_v5(&source),
        6 => atto_compiler::compile_future_int_aot(&source),
        _ => atto_compiler::compile(&source),
    };
    match result {
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
