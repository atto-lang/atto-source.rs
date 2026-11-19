//! Native host for the same compiler core: `attoc input.atto [-o out.wasm]`.
//! Used for native-versus-browser differential checks.
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (input, output, check_only) = match args.as_slice() {
        [flag] if flag == "--help" || flag == "-h" => {
            println!("usage: attoc <input.atto> [-o out.wasm]\n       attoc --check <input.atto>");
            return ExitCode::SUCCESS;
        }
        [flag, i] if flag == "--check" => (i.clone(), None, true),
        [i] => (i.clone(), None, false),
        [i, o, out] if o == "-o" => (i.clone(), Some(out.clone()), false),
        _ => {
            eprintln!("usage: attoc <input.atto> [-o out.wasm]\n       attoc --check <input.atto>");
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
    // Treat source paths as logical paths relative to the invocation's
    // package working directory. Absolute paths outside that directory have
    // no implicit namespace root and must not leak host directory names.
    let source_path = std::path::Path::new(&input);
    let relative_path = if source_path.is_absolute() {
        std::env::current_dir().ok()
            .and_then(|root| source_path.strip_prefix(root).ok().map(|path| path.to_path_buf()))
    } else {
        Some(source_path.to_path_buf())
    };
    let result = match relative_path {
        Some(ref name) => {
            // The native CLI is the only filesystem authority. The compiler
            // accepts an injected loader and never reads host paths itself.
            // Canonicalization rejects symlink escapes from this project root.
            match std::env::current_dir().and_then(|p| p.canonicalize()) {
                Ok(root) => atto_compiler::compile_with_imports(&source, name.as_path(), |path| {
                    let resolved = root.join(path).canonicalize()?;
                    if !resolved.starts_with(&root) {
                        return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied,
                            "import resolves outside project root"));
                    }
                    std::fs::read_to_string(resolved)
                }),
                Err(_) => atto_compiler::compile_with_path(&source, name.as_path()),
            }
        }
        None => atto_compiler::compile(&source),
    };
    match result {
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
