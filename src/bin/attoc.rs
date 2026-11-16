//! Native host for the same compiler core; no implicit environment version.
//! Usage: attoc input.atto [-o out.wasm] [--language-version 1|2].
//! The existing unflagged v2 behavior is a draft compatibility decision.
use atto_compiler::{compile_with_language_version, LanguageVersion};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut input: Option<String> = None;
    let mut output: Option<String> = None;
    let mut version = LanguageVersion::V2;
    let mut version_explicit = false;
    let mut index = 0usize;
    while index < args.len() {
        let arg = &args[index];
        if arg == "-o" {
            index += 1;
            if output.is_some() || index >= args.len() || args[index].starts_with('-') {
                return usage();
            }
            output = Some(args[index].clone());
        } else if arg == "--language-version" || arg.starts_with("--language-version=") {
            if version_explicit { return usage(); }
            let number = if arg == "--language-version" {
                index += 1;
                match args.get(index) { Some(number) => number.as_str(), None => return usage() }
            } else {
                &arg["--language-version=".len()..]
            };
            version = match number {
                "1" => LanguageVersion::V1,
                "2" => LanguageVersion::V2,
                _ => return usage(),
            };
            version_explicit = true;
        } else if arg.starts_with('-') || input.is_some() {
            return usage();
        } else {
            input = Some(arg.clone());
        }
        index += 1;
    }
    let Some(input) = input else { return usage(); };
    let source = match std::fs::read(&input).map(String::from_utf8) {
        Ok(Ok(s)) => s,
        _ => {
            eprintln!("{input}: cannot read UTF-8 source");
            return ExitCode::from(2);
        }
    };
    match compile_with_language_version(&source, version) {
        Ok(bytes) => {
            if let Some(path) = output {
                if std::fs::write(&path, &bytes).is_err() {
                    eprintln!("{path}: cannot write");
                    return ExitCode::from(2);
                }
            } else {
                use std::io::Write;
                if std::io::stdout().write_all(&bytes).is_err() {
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

fn usage() -> ExitCode {
    eprintln!("usage: attoc <input.atto> [-o out.wasm] [--language-version 1|2]");
    ExitCode::from(2)
}
