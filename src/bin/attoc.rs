//! Native host for the same compiler core: `attoc input.atto [-o out.wasm]`.
//! Used for native-versus-browser differential checks.
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // 0: production, 1: stateless preview, 2: bound actor-instance preview.
    let (input, output, preview_mode) = match args.as_slice() {
        [i] => (i.clone(), None,0),
        [i, o, out] if o == "-o" => (i.clone(),Some(out.clone()),0),
        [flag, i] if flag == "--scalar-preview" => (i.clone(),None,1),
        [flag, i, o, out] if flag == "--scalar-preview" && o == "-o" =>
            (i.clone(),Some(out.clone()),1),
        [flag, i] if flag == "--receiver-instance-preview" => (i.clone(),None,2),
        [flag, i, o, out] if flag == "--receiver-instance-preview" && o == "-o" =>
            (i.clone(),Some(out.clone()),2),
        _ => {
            eprintln!("usage: attoc [--scalar-preview | --receiver-instance-preview] <input.atto> [-o out.wasm]");
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
    let built=match preview_mode {
        1 => atto_compiler::target_scalar::compile_scalar_receiver_preview(&source),
        2 => atto_compiler::target_scalar::compile_scalar_receiver_instance_preview(&source),
        _ => atto_compiler::compile(&source),
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
