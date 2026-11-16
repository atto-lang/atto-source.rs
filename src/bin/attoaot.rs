//! Standalone offline AOT packager: AttoLang -> guest Wasm -> host-native .cwasm.
//!
//! This is a *build-time* compiler. Runtime loads precompiled machine code with
//! a compatible Wasmtime host. No AttoLang parsing or Wasm JIT is needed then.
//! The result is NOT a standalone ELF/Mach-O/PE executable or a portable Wasm.
use std::{
    env, ffi::OsStr, fs, io, path::{Path, PathBuf}, process::{Command, ExitCode},
};

fn usage() -> &'static str {
    "usage: attoaot input.atto -o output.cwasm [--wasmtime PATH] [--target TRIPLE]\n\
     output is a trusted, host/Wasmtime-version-specific precompiled module\n\
     this first native AOT profile supports only import-free AttoLang guest code"
}

struct Options {
    input: PathBuf,
    output: PathBuf,
    wasmtime: PathBuf,
    target: Option<String>,
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Options, String> {
    let mut args = args.into_iter();
    let Some(input) = args.next() else { return Err(usage().into()); };
    if input.starts_with('-') { return Err(usage().into()); }
    let mut output = None;
    let mut wasmtime = PathBuf::from("wasmtime");
    let mut target = None;
    while let Some(flag) = args.next() {
        let value = args.next().ok_or_else(|| format!("missing value after {flag}\n{}", usage()))?;
        match flag.as_str() {
            "-o" if output.is_none() => output = Some(PathBuf::from(value)),
            "--wasmtime" => wasmtime = PathBuf::from(value),
            "--target" if target.is_none() => {
                // Reject option injection and invalid target spellings.
                if !value.contains('-') || value.starts_with('-') ||
                    !value.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)) {
                    return Err("invalid target triple".into());
                }
                target = Some(value);
            }
            _ => return Err(format!("unknown or repeated option: {flag}\n{}", usage())),
        }
    }
    let output = output.ok_or_else(|| usage().to_string())?;
    if output.extension() != Some(OsStr::new("cwasm")) {
        return Err("native AOT output must end in .cwasm (not an executable)".into());
    }
    if Path::new(&input).extension() != Some(OsStr::new("atto")) {
        return Err("input must be an .atto source file".into());
    }
    Ok(Options { input: input.into(), output, wasmtime, target })
}

fn uleb32(data: &[u8], cursor: &mut usize) -> Result<u32, String> {
    let mut result = 0u32;
    for shift in (0..=28).step_by(7) {
        let byte = *data.get(*cursor).ok_or("truncated Wasm LEB128")?;
        *cursor += 1;
        if shift == 28 && byte > 0x0f { return Err("oversized Wasm LEB128".into()); }
        result |= u32::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 { return Ok(result); }
    }
    Err("oversized Wasm LEB128".into())
}

/// Only precompile modules whose entire host import set can be bound by a
/// source-free Wasmtime CLI. Actor/stdio imports currently require a dedicated
/// AOT runtime host; do not silently create an uninstantiable artifact.
fn import_count(bytes: &[u8]) -> Result<u32, String> {
    if bytes.get(..8) != Some(&b"\0asm\x01\0\0\0"[..]) {
        return Err("invalid Wasm header".into());
    }
    let mut at = 8;
    let mut count = 0u32;
    while at < bytes.len() {
        let id = bytes[at];
        at += 1;
        let size = usize::try_from(uleb32(bytes, &mut at)?)
            .map_err(|_| "Wasm section overflow")?;
        let end = at.checked_add(size).filter(|end| *end <= bytes.len())
            .ok_or("truncated Wasm section")?;
        if id == 2 {
            let mut pos = at;
            count = uleb32(bytes, &mut pos)?;
            if pos > end { return Err("truncated Wasm import section".into()); }
            break;
        }
        at = end;
    }
    Ok(count)
}

/// A temporary build directory inside the output filesystem. A hard link from
/// a complete .cwasm file is used for atomic, exclusive final publication.
struct Stage(PathBuf);
impl Stage {
    fn new(output: &Path) -> io::Result<Self> {
        let parent = output.parent().filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        for i in 0..1000 {
            let candidate = parent.join(format!(".atto-aot-stage-{}-{i}", std::process::id()));
            match fs::create_dir(&candidate) {
                Ok(()) => return Ok(Self(candidate)),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
        Err(io::Error::new(io::ErrorKind::AlreadyExists, "no free temporary stage"))
    }
}
impl Drop for Stage {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}

fn run(options: Options) -> Result<(), String> {
    if options.output.exists() {
        return Err(format!("refusing to overwrite output: {}", options.output.display()));
    }
    let source = fs::read_to_string(&options.input)
        .map_err(|e| format!("cannot read {}: {e}", options.input.display()))?;
    let guest = atto_compiler::compile(&source)
        .map_err(|e| format!("AttoLang compilation failed: {e}"))?;
    let count = import_count(&guest)?;
    if count != 0 {
        return Err(format!(
            "E_AOT_HOST_IMPORTS: guest requires {count} non-WASI host imports; \
             build an explicit AOT runtime host before native precompilation"
        ));
    }
    let stage = Stage::new(&options.output)
        .map_err(|e| format!("cannot create AOT staging directory: {e}"))?;
    let intermediate = stage.0.join("guest.wasm");
    let native = stage.0.join("guest.cwasm");
    fs::write(&intermediate, &guest).map_err(|e| format!("cannot stage Wasm: {e}"))?;
    let mut cmd = Command::new(&options.wasmtime);
    cmd.arg("compile");
    if let Some(triple) = &options.target {
        cmd.arg("--target").arg(triple);
    }
    cmd.arg("-o").arg(&native).arg(&intermediate);
    let status = cmd.status().map_err(|e| format!(
        "cannot start Wasmtime AOT compiler at {}: {e}", options.wasmtime.display()
    ))?;
    if !status.success() {
        return Err(format!("Wasmtime AOT compilation failed (status: {status})"));
    }
    let size = fs::metadata(&native)
        .map_err(|e| format!("AOT compiler did not create a native artifact: {e}"))?.len();
    if size == 0 { return Err("AOT compiler produced an empty artifact".into()); }
    // Both files are on the same filesystem, and hard_link refuses to replace.
    fs::hard_link(&native, &options.output)
        .map_err(|e| format!("cannot publish completed AOT artifact exclusively: {e}"))?;
    println!(
        "AOT native module: {} ({size} bytes); load with a compatible Wasmtime runtime",
        options.output.display()
    );
    Ok(())
}

fn main() -> ExitCode {
    if env::args().skip(1).eq(["--help".to_string()].into_iter()) {
        println!("{}", usage());
        return ExitCode::SUCCESS;
    }
    let opts = match parse_args(env::args().skip(1)) {
        Ok(opts) => opts,
        Err(reason) => { eprintln!("{reason}"); return ExitCode::from(2); }
    };
    match run(opts) {
        Ok(()) => ExitCode::SUCCESS,
        Err(reason) => { eprintln!("{reason}"); ExitCode::FAILURE }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_require_explicit_aot_output_and_safe_target() {
        assert!(parse_args(["input.atto".into(), "-o".into(), "out.wasm".into()]).is_err());
        assert!(parse_args(["input.atto".into(), "-o".into(), "out.cwasm".into(),
            "--target".into(), "--evil".into()]).is_err());
        assert!(parse_args(["input.atto".into(), "-o".into(), "out.cwasm".into(),
            "--target".into(), "x86_64-unknown-linux-gnu".into()]).is_ok());
    }

    #[test]
    fn imported_guest_cannot_be_published_as_hostless_native_aot() {
        let guest = atto_compiler::compile(
            "pub fnc main(): int { stdio.println(\"hello\"); return 0; }"
        ).unwrap();
        assert!(import_count(&guest).unwrap() > 0);
        let pure = atto_compiler::compile(
            "pub fnc main(): int { val a = [10,20,30]; return a[-1]; }"
        ).unwrap();
        assert_eq!(import_count(&pure), Ok(0));
    }

    #[test]
    fn truncated_wasm_metadata_never_looks_import_free() {
        assert!(import_count(&[0, 97, 115]).is_err());
        assert!(import_count(b"\0asm\x01\0\0\0\x02\x03\x81").is_err());
        assert_eq!(import_count(b"\0asm\x01\0\0\0"), Ok(0));
    }
}
