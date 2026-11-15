//! Native host for the same compiler core: `attoc input.atto [-o out.wasm]`.
//! Used for native-versus-browser differential checks.
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (input, output, scalar_preview) = match args.as_slice() {
        [i] => (i.clone(), None,false),
        [i, o, out] if o == "-o" => (i.clone(), Some(out.clone()),false),
        [flag, i] if flag == "--scalar-preview" => (i.clone(),None,true),
        [flag, i, o, out] if flag == "--scalar-preview" && o == "-o" =>
            (i.clone(),Some(out.clone()),true),
        _ => {
            eprintln!("usage: attoc [--scalar-preview] <input.atto> [-o out.wasm]");
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
    let built=if scalar_preview {
        atto_compiler::target_scalar::compile_scalar_receiver_preview(&source)
    } else {
        atto_compiler::compile(&source)
    };
    match built {
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
