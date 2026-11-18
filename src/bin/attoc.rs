//! Native host for the common Atto compiler core.
//! Legacy language version 1 is the default. Opt-in strict body-mode v2:
//! `attoc --language-version 2 input.atto [-o out.wasm]`.
use std::process::ExitCode;
use atto_compiler::parser::LanguageVersion;

enum CliError {
    Usage(String),
    Compile(String),
}
impl From<&str> for CliError {
    fn from(message: &str) -> Self { Self::Usage(message.to_string()) }
}
impl From<String> for CliError {
    fn from(message: String) -> Self { Self::Usage(message) }
}

fn run() -> Result<(), CliError> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut input: Option<String> = None;
    let mut output: Option<String> = None;
    let mut version = LanguageVersion::Legacy;
    let mut version_seen = false;
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--language-version" || arg.starts_with("--language-version=") {
            if version_seen { return Err("duplicate --language-version".into()); }
            version_seen = true;
            let num = if arg == "--language-version" {
                i += 1;
                args.get(i).ok_or("missing --language-version value")?.as_str()
            } else {
                arg.strip_prefix("--language-version=").unwrap()
            };
            version = match num {
                "1" => LanguageVersion::Legacy,
                "2" => LanguageVersion::Explicit,
                _ => return Err(format!("unsupported language version '{num}'; choose 1 or 2").into()),
            };
        } else if arg == "-o" {
            if output.is_some() { return Err("duplicate -o output".into()); }
            i += 1;
            output = Some(args.get(i).ok_or("missing -o output path")?.clone());
        } else if arg.starts_with('-') {
            return Err(format!("unknown option '{arg}'").into());
        } else if input.replace(arg.clone()).is_some() {
            return Err("expected exactly one input.atto file".into());
        }
        i += 1;
    }
    let input = input.ok_or("missing input.atto file")?;
    let source = std::fs::read(&input).map_err(|_| format!("{input}: cannot read UTF-8 source"))?;
    let source = String::from_utf8(source).map_err(|_| format!("{input}: cannot read UTF-8 source"))?;
    let bytes = atto_compiler::compile_with_version(&source, version)
        .map_err(|d| CliError::Compile(format!("{input}:{d}")))?;
    if let Some(path) = output {
        std::fs::write(&path, &bytes).map_err(|_| format!("{path}: cannot write"))?;
    } else {
        use std::io::Write;
        std::io::stdout().write_all(&bytes).map_err(|_| "cannot write output".to_string())?;
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(CliError::Compile(err)) => { eprintln!("{err}"); ExitCode::from(1) }
        Err(CliError::Usage(err)) => {
            eprintln!("{err}");
            eprintln!("usage: attoc [--language-version 1|2] <input.atto> [-o out.wasm]");
            ExitCode::from(2)
        }
    }
}
