//! Native host for the same compiler core; no implicit environment version.
//! Usage: attoc input.atto [-o out.wasm] [--language-version 1|2].
//! The existing unflagged v2 behavior is a draft compatibility decision.
use atto_compiler::{compile_with_language_version, LanguageVersion, SOURCE_CAP};
use std::{io::Read, process::ExitCode};

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
                _ => {
                    eprintln!("E_LANGUAGE_VERSION: unsupported language version (expected 1 or 2)");
                    return ExitCode::from(2);
                },
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
    // Read at most one byte past the compiler's source cap. Reading the full
    // file first would allow an untrusted path to exhaust host memory before
    // the compiler's SOURCE_CAP diagnostic could protect the process.
    let file = match std::fs::File::open(&input) {
        Ok(file) => file,
        Err(_) => {
            eprintln!("{input}: cannot read UTF-8 source");
            return ExitCode::from(2);
        }
    };
    let mut bytes = Vec::new();
    if file.take((SOURCE_CAP as u64) + 1).read_to_end(&mut bytes).is_err() {
        eprintln!("{input}: cannot read UTF-8 source");
        return ExitCode::from(2);
    }
    if bytes.len() > SOURCE_CAP {
        eprintln!("{input}:1:1: E_SOURCE_LIMIT: source exceeds {SOURCE_CAP} UTF-8 bytes");
        return ExitCode::from(1);
    }
    let source = match String::from_utf8(bytes) {
        Ok(source) => source,
        Err(_) => {
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
