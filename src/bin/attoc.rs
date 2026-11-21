//! Native CLI for browser-guest Wasm compilation.
//! Single source: attoc input.atto [-o out.wasm]
//! Syntax check: attoc --check input.atto
//! Linked package: attoc --package-root DIR --entry src/main.atto [-o out.wasm]
//! Discovered package: attoc --package path/to/main.atto [-o out.wasm]
use std::path::Path;
use std::process::ExitCode;
use std::io::Write;

// The .atto source must never be removed when a user accidentally chooses it
// (or its symlink/hardlink alias) as the Wasm destination.
fn same_file(left: &Path, right: &Path) -> bool {
    if left == right { return true; }
    if let (Ok(a), Ok(b)) = (left.canonicalize(), right.canonicalize()) {
        if a == b { return true; }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let (Ok(a), Ok(b)) = (left.metadata(), right.metadata()) {
            if a.dev() == b.dev() && a.ino() == b.ino() { return true; }
        }
    }
    false
}

fn input_source_path(input: &Input) -> std::path::PathBuf {
    match input {
        Input::Single(file) | Input::Check(file) | Input::Discovered(file) =>
            Path::new(file).to_path_buf(),
        Input::Package { root, entry } | Input::LockedPackage { root, entry, .. } =>
            Path::new(root).join(entry),
    }
}

// No previous successful guest is allowed to survive a failed recompilation.
// This is deliberately before source reading, parsing and code generation.
fn clear_previous_output(input: &Input, output: &str) -> std::io::Result<()> {
    if same_file(&input_source_path(input), Path::new(output)) {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput,
            "output resolves to the Atto source file"));
    }
    match std::fs::remove_file(output) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

// Creation is exclusive: a planted output symlink cannot be followed between
// the initial stale-file cleanup and successful compiler output publication.
fn write_fresh_wasm(output: &str, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .write(true).create_new(true).open(output)?;
    file.write_all(bytes)?;
    file.sync_all()
}

fn publish_wasm(output: &str, bytes: &[u8]) -> std::io::Result<()> {
    let result = write_fresh_wasm(output, bytes);
    if result.is_err() { let _ = std::fs::remove_file(output); }
    result
}

enum Input {
    Single(String),
    Check(String),
    Discovered(String),
    Package { root: String, entry: String },
    LockedPackage { root: String, entry: String, digest: String },
}

fn usage() -> ExitCode {
    eprintln!("usage: attoc <input.atto> [-o out.wasm]\n       attoc --check <input.atto>\n       attoc --package <input.atto> [-o out.wasm]\n       attoc --package-root DIR --entry relative.atto [-o out.wasm]\n       attoc --package-root DIR --entry relative.atto --trusted-lock-sha256 DIGEST [-o out.wasm]");
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if matches!(args.as_slice(), [flag] if flag == "--help" || flag == "-h") {
        println!("usage: attoc <input.atto> [-o out.wasm]\n       attoc --check <input.atto>\n       attoc --package <input.atto> [-o out.wasm]\n       attoc --package-root DIR --entry relative.atto [-o out.wasm]");
        return ExitCode::SUCCESS;
    }
    let (input, output) = match args.as_slice() {
        [flag, entry] if flag == "--check" => (Input::Check(entry.clone()), None),
        [i] if !i.starts_with('-') => (Input::Single(i.clone()), None),
        [i, o, out] if !i.starts_with('-') && o == "-o" => (Input::Single(i.clone()), Some(out.clone())),
        [flag, entry] if flag == "--package" => (Input::Discovered(entry.clone()), None),
        [flag, entry, o, out] if flag == "--package" && o == "-o" =>
            (Input::Discovered(entry.clone()), Some(out.clone())),
        [p, root, e, entry] if p == "--package-root" && e == "--entry" =>
            (Input::Package { root: root.clone(), entry: entry.clone() }, None),
        [p, root, e, entry, o, out]
            if p == "--package-root" && e == "--entry" && o == "-o" =>
            (Input::Package { root: root.clone(), entry: entry.clone() }, Some(out.clone())),
        [p, root, e, entry, flag, digest]
            if p == "--package-root" && e == "--entry" && flag == "--trusted-lock-sha256" =>
            (Input::LockedPackage { root: root.clone(), entry: entry.clone(), digest: digest.clone() }, None),
        [p, root, e, entry, flag, digest, o, out]
            if p == "--package-root" && e == "--entry" && flag == "--trusted-lock-sha256" && o == "-o" =>
            (Input::LockedPackage { root: root.clone(), entry: entry.clone(), digest: digest.clone() }, Some(out.clone())),
        _ => return usage(),
    };
    if let Some(ref target) = output {
        if let Err(error) = clear_previous_output(&input, target) {
            eprintln!("{target}: cannot prepare Wasm output: {error}");
            return ExitCode::from(2);
        }
    }
    let check_only = matches!(&input, Input::Check(_));
    let (label, compiled) = match input {
        Input::Single(path) | Input::Check(path) => {
            let source = match std::fs::read(&path).map(String::from_utf8) {
                Ok(Ok(s)) => s,
                _ => {
                    eprintln!("{path}: cannot read UTF-8 source");
                    return ExitCode::from(2);
                }
            };
            // A caller-relative logical path enables use default namespace(N)
            // without leaking the host's absolute directory into symbol IDs.
            let source_path = Path::new(&path);
            let relative = if source_path.is_absolute() {
                std::env::current_dir().ok().and_then(|cwd|
                    source_path.strip_prefix(cwd).ok().map(|p| p.to_path_buf()))
            } else { Some(source_path.to_path_buf()) };
            let result = match relative {
                Some(name) => match std::env::current_dir().and_then(|p| p.canonicalize()) {
                    Ok(root) => atto_compiler::compile_with_imports(&source, name.as_path(), |path| {
                        // Restrict file imports to canonical paths inside this project root.
                        let resolved = root.join(path).canonicalize()?;
                        if !resolved.starts_with(&root) {
                            return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied,
                                "import resolves outside project root"));
                        }
                        std::fs::read_to_string(resolved)
                    }),
                    Err(_) => atto_compiler::compile_with_path(&source, name.as_path()),
                },
                None => atto_compiler::compile(&source),
            };
            (path, result)
        }
        Input::Discovered(path) => {
            let label = path.clone();
            (label, atto_compiler::package_resolver::compile_discovered(Path::new(&path)))
        }
        Input::Package { root, entry } => {
            let label = format!("{root}/{entry}");
            (label, atto_compiler::package_resolver::compile_package(Path::new(&root), &entry))
        }
        Input::LockedPackage { root, entry, digest } => {
            let label = format!("{root}/{entry}");
            (label, atto_compiler::package_resolver::compile_package_locked(
                Path::new(&root), &entry, &digest))
        }
    };
    match compiled {
        Ok(bytes) => {
            if check_only { return ExitCode::SUCCESS; }
            if let Some(output) = output {
                if let Err(error) = publish_wasm(&output, &bytes) {
                    eprintln!("{output}: cannot write Wasm output: {error}");
                    return ExitCode::from(2);
                }
            } else {
                if std::io::stdout().write_all(&bytes).is_err() {
                    eprintln!("cannot write compiled Wasm to stdout");
                    return ExitCode::from(2);
                }
            }
            ExitCode::SUCCESS
        }
        Err(d) => {
            eprintln!("{label}: {d}");
            ExitCode::from(1)
        }
    }
}
