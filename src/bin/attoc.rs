//! Native CLI for browser-guest Wasm compilation.
//! Single file: attoc input.atto [-o out.wasm]
//! Checked package: attoc --package-root DIR --entry src/main.atto [-o out.wasm]
use std::process::ExitCode;
use std::path::Path;

enum Input {
    Single(String),
    Package { root: String, entry: String },
}

fn usage() -> ExitCode {
    eprintln!(
        "usage: attoc <input.atto> [-o out.wasm]\n       attoc --package-root <dir> --entry <relative.atto> [-o out.wasm]"
    );
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (input, output) = match args.as_slice() {
        [i] if !i.starts_with('-') => (Input::Single(i.clone()), None),
        [i, o, out] if !i.starts_with('-') && o == "-o" =>
            (Input::Single(i.clone()), Some(out.clone())),
        [p, root, e, entry] if p == "--package-root" && e == "--entry" =>
            (Input::Package { root: root.clone(), entry: entry.clone() }, None),
        [p, root, e, entry, o, out]
            if p == "--package-root" && e == "--entry" && o == "-o" =>
            (Input::Package { root: root.clone(), entry: entry.clone() }, Some(out.clone())),
        _ => return usage(),
    };
    let (label, compiled) = match input {
        Input::Single(path) => {
            let source = match std::fs::read(&path).map(String::from_utf8) {
                Ok(Ok(s)) => s,
                _ => {
                    eprintln!("{path}: cannot read UTF-8 source");
                    return ExitCode::from(2);
                }
            };
            (path, atto_compiler::compile(&source))
        }
        Input::Package { root, entry } => {
            let label = format!("{root}/{entry}");
            (label, atto_compiler::package_resolver::compile_package(Path::new(&root), &entry))
        }
    };
    match compiled {
        Ok(bytes) => {
            if let Some(output) = output {
                if std::fs::write(&output, &bytes).is_err() {
                    eprintln!("{output}: cannot write Wasm");
                    return ExitCode::from(2);
                }
            } else {
                use std::io::Write;
                if std::io::stdout().write_all(&bytes).is_err() {
                    eprintln!("cannot write Wasm to stdout");
                    return ExitCode::from(2);
                }
            }
            ExitCode::SUCCESS
        }
        Err(diagnostic) => {
            eprintln!("{label}: {diagnostic}");
            ExitCode::from(1)
        }
    }
}
