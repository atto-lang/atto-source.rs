//! Native CLI for browser-guest Wasm compilation.
//! Single source: attoc input.atto [-o out.wasm]
//! Syntax check: attoc --check input.atto
//! Linked package: attoc --package-root DIR --entry src/main.atto [-o out.wasm]
//! Discovered package: attoc --package path/to/main.atto [-o out.wasm]
//! Borrow conformance (single source only), extra flags:
//!   --borrow-report OUT.jsonl --compiler-git-sha <40 hex> [--case ID]
//!   writes atto-borrow-backend-report-v1 lines for atto-borrow-conform cross-check.
use std::io::Read;
use std::path::Path;
use std::process::ExitCode;

enum Input {
    Single(String),
    Check(String),
    Discovered(String),
    Package { root: String, entry: String },
    LockedPackage { root: String, entry: String, digest: String },
}

fn usage() -> ExitCode {
    eprintln!("usage: attoc <input.atto> [-o out.wasm]\n       attoc --durable-scalar-v1 <input.atto> -o out.wasm\n       attoc --durable-counter-actor-v1 <input.atto> -o out.wasm\n       attoc --check <input.atto>\n       attoc --package <input.atto> [-o out.wasm]\n       attoc --package-root DIR --entry relative.atto [-o out.wasm]\n       attoc --package-root DIR --entry relative.atto --trusted-lock-sha256 DIGEST [-o out.wasm]");
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    // Separate explicit ABI, no package/import lookup or borrow-report modes.
    if raw.first().is_some_and(|a| a == "--durable-scalar-v1" || a == "--durable-counter-actor-v1") {
        let [_, input, flag, output] = raw.as_slice() else { return usage(); };
        if flag != "-o" { return usage(); }
        let mut source = String::new();
        if let Err(e) = std::fs::File::open(input).and_then(|f|
            f.take((atto_compiler::SOURCE_CAP + 1) as u64).read_to_string(&mut source)) {
            eprintln!("{input}: {e}");
            return ExitCode::from(2);
        }
        let compiled = if raw[0] == "--durable-counter-actor-v1" {
            atto_compiler::durable_counter_actor::compile(&source)
        } else {
            atto_compiler::compile_durable_scalar(&source)
        };
        return match compiled {
            Ok(bytes) => match std::fs::write(output, bytes) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => { eprintln!("{output}: {e}"); ExitCode::from(2) }
            },
            Err(d) => { eprintln!("{input}: {d}"); ExitCode::from(1) }
        };
    }
    let mut args = Vec::new();
    let (mut report_path, mut compiler_sha, mut case) = (None, None, None);
    let mut it = raw.into_iter();
    while let Some(a) = it.next() {
        if let Some(os) = a.strip_prefix("--target-os=") {
            // Selects platform-variant library units such as std/path.
            match os {
                "windows" => atto_compiler::library::set_target_os(atto_compiler::library::TargetOs::Windows),
                "linux" | "macos" | "freebsd" | "posix" =>
                    atto_compiler::library::set_target_os(atto_compiler::library::TargetOs::Posix),
                _ => {
                    eprintln!("unknown --target-os (expected linux|macos|freebsd|posix|windows)");
                    return ExitCode::from(2);
                }
            }
            continue;
        }
        let slot = match a.as_str() {
            "--borrow-report" => &mut report_path,
            "--compiler-git-sha" => &mut compiler_sha,
            "--case" => &mut case,
            _ => { args.push(a); continue; }
        };
        match it.next() {
            Some(v) => *slot = Some(v),
            None => return usage(),
        }
    }
    if report_path.is_some() {
        let sha_ok = compiler_sha.as_deref().is_some_and(|s| s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit()));
        if !sha_ok {
            eprintln!("--borrow-report requires --compiler-git-sha <40 hex>");
            return ExitCode::from(2);
        }
        atto_compiler::borrow_export::begin();
    } else if compiler_sha.is_some() || case.is_some() {
        return usage();
    }
    let mut report_source: Option<Vec<u8>> = None;
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
            if report_path.is_some() { report_source = Some(source.as_bytes().to_vec()); }
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
                        // Read at most SOURCE_CAP+1 bytes: importing a huge
                        // dependency must never allocate its entire contents
                        // before the lexer can enforce the byte admission cap.
                        let mut data = String::new();
                        std::fs::File::open(resolved)?
                            .take((atto_compiler::SOURCE_CAP + 1) as u64)
                            .read_to_string(&mut data)?;
                        if data.len() > atto_compiler::SOURCE_CAP {
                            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData,
                                "imported source exceeds compiler byte limit"));
                        }
                        Ok(data)
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
    if let Some(out) = &report_path {
        let Some(source) = &report_source else {
            eprintln!("--borrow-report supports single-source compiles only");
            return ExitCode::from(2);
        };
        let mut records = atto_compiler::borrow_export::finish();
        if let Err(d) = &compiled {
            if !records.iter().any(|r| r.local.is_some()) {
                records.push(atto_compiler::borrow_export::Record {
                    function: "<module>".to_string(),
                    local: Some((d.code.to_string(), d.message.clone())),
                    mir: Err("rejected before per-function ownership analysis".to_string()),
                    prefix_at_error: false,
                });
            }
        }
        let digest = atto_compiler::sha256::hex_digest(source);
        let ctx = atto_compiler::borrow_export::ReportContext {
            compiler_git_sha: compiler_sha.as_deref().unwrap_or_default(),
            source_sha256: &digest,
            source_path: &label,
            case: case.as_deref(),
        };
        if std::fs::write(out, atto_compiler::borrow_export::report_lines(&records, &ctx)).is_err() {
            eprintln!("{out}: cannot write borrow report");
            return ExitCode::from(2);
        }
    }
    match compiled {
        Ok(bytes) => {
            if check_only { return ExitCode::SUCCESS; }
            if let Some(output) = output {
                if std::fs::write(&output, &bytes).is_err() {
                    eprintln!("{output}: cannot write Wasm output");
                    return ExitCode::from(2);
                }
            } else {
                use std::io::Write;
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
