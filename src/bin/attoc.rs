//! Native host for the exact same compiler core used by browser wasm32.
//! attoc input.atto [-o out.wasm] [--ownership-profile legacy|borrow-default-v1]
//! No package manifest is silently discovered here; the invoking build tool
//! must explicitly select and verify the package's versioned ownership mode.
use atto_compiler::OwnershipProfile;
use std::process::ExitCode;

fn usage() -> ExitCode {
    eprintln!("usage: attoc <input.atto> [-o out.wasm] [--ownership-profile legacy|borrow-default-v1]");
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let mut input = None;
    let mut output = None;
    let mut profile = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "-o" {
            if output.is_some() { return usage(); }
            match args.next() {
                Some(value) if !value.is_empty() => output = Some(value),
                _ => return usage(),
            }
        } else if arg == "--ownership-profile" || arg.starts_with("--ownership-profile=") {
            if profile.is_some() { return usage(); }
            let value = if arg == "--ownership-profile" {
                match args.next() { Some(value) => value, None => return usage() }
            } else {
                arg.trim_start_matches("--ownership-profile=").to_string()
            };
            profile = match value.as_str() {
                "legacy" => Some(OwnershipProfile::Legacy),
                "borrow-default-v1" => Some(OwnershipProfile::BorrowDefaultV1),
                _ => return usage(),
            };
        } else if arg.starts_with('-') || input.is_some() {
            return usage();
        } else {
            input = Some(arg);
        }
    }
    let Some(input) = input else { return usage() };
    let source = match std::fs::read(&input).map(String::from_utf8) {
        Ok(Ok(s)) => s,
        _ => {
            eprintln!("{input}: cannot read UTF-8 source");
            return ExitCode::from(2);
        }
    };
    let result = match profile {
        Some(profile) => atto_compiler::compile_with_profile(&source, profile),
        None => atto_compiler::compile(&source),
    };
    match result {
        Ok(bytes) => {
            if let Some(out) = output {
                if std::fs::write(&out, &bytes).is_err() {
                    eprintln!("{out}: cannot write");
                    return ExitCode::from(2);
                }
            } else {
                use std::io::Write;
                if std::io::stdout().write_all(&bytes).is_err() {
                    eprintln!("{input}: cannot write output");
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
