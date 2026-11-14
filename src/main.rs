use oreslang_source_rs::{
    cfg, codegen, effects, flow, frontend, mir,
    profile::{self, BuildProfile},
    safety,
};
use std::{env, fs, io::Read, path::Path, process::ExitCode};

const MAX_SOURCE_BYTES: usize = 1024 * 1024;

fn canonical_destination(path: &str) -> Result<std::path::PathBuf, String> {
    let destination = Path::new(path);
    // Existing symlinks, including dangling ones, cannot impersonate a new
    // output path. Canonicalization must succeed for a present directory entry.
    if fs::symlink_metadata(destination).is_ok() {
        return fs::canonicalize(destination).map_err(|e| e.to_string());
    }
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let directory = fs::canonicalize(parent).map_err(|e| e.to_string())?;
    let basename = destination
        .file_name()
        .ok_or("output requires a filename")?;
    Ok(directory.join(basename))
}

fn main() -> ExitCode {
    let raw_args: Vec<String> = env::args().collect();
    let result: Result<(), String> = (|| {
        let mut profile = BuildProfile::Dev;
        let mut profile_seen = false;
        let mut freestanding = false;
        let mut checked_i64 = false;
        let mut args = Vec::with_capacity(raw_args.len());
        args.push(raw_args[0].clone());

        for arg in raw_args.iter().skip(1) {
            if let Some(selected) = BuildProfile::parse_flag(arg) {
                if profile_seen {
                    return Err(
                        "--release, --strict-aot and --hard-rt are mutually exclusive".into(),
                    );
                }
                profile = selected;
                profile_seen = true;
            } else if arg == "--freestanding" {
                if freestanding {
                    return Err("duplicate --freestanding".into());
                }
                freestanding = true;
            } else if arg == "--checked-i64" {
                if checked_i64 {
                    return Err("duplicate --checked-i64".into());
                }
                checked_i64 = true;
            } else {
                args.push(arg.clone());
            }
        }

        // Independent Rust source-ID verification runs on THIS compiler's parsed
        // and type-checked MIR and emits a paired native object plus sidecar.
        // It deliberately does not claim logging syntax, probe insertion or JIT.
        if let [_, flag, input, registry, source_unit, object, map] = args.as_slice() {
            if flag == "--compile-with-source-ids" {
                if profile_seen || freestanding || checked_i64 {
                    return Err("source-ID object mode currently supports default scalar profile only".into());
                }
                // Only real regular source/registry files can enter this mode;
                // neither FIFOs nor symlink leaf substitutions may park the compiler.
                let bounded_read = |path: &str| -> Result<String, String> {
                    let before = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
                    if !before.is_file() {
                        return Err("source-ID input must be a regular non-symlink file".into());
                    }
                    let mut opts = fs::OpenOptions::new();
                    opts.read(true);
                    #[cfg(unix)] {
                        use std::os::unix::fs::OpenOptionsExt;
                        opts.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC);
                    }
                    let mut file = opts.open(path).map_err(|e| e.to_string())?;
                    let opened = file.metadata().map_err(|e| e.to_string())?;
                    if !opened.is_file() {
                        return Err("source-ID input changed to a nonregular file".into());
                    }
                    #[cfg(unix)] {
                        use std::os::unix::fs::MetadataExt;
                        if opened.dev() != before.dev() || opened.ino() != before.ino() {
                            return Err("source-ID input changed before opening".into());
                        }
                    }
                    let mut data = String::new();
                    file.take(MAX_SOURCE_BYTES as u64 + 1)
                        .read_to_string(&mut data).map_err(|e| e.to_string())?;
                    if data.len() > MAX_SOURCE_BYTES {
                        return Err("source/registry exceeds 1 MiB admission budget".into());
                    }
                    Ok(data)
                };
                let input_id = fs::canonicalize(input).map_err(|e| e.to_string())?;
                let registry_id = fs::canonicalize(registry).map_err(|e| e.to_string())?;
                let package_root = fs::canonicalize(".").map_err(|e| e.to_string())?;
                // In this scalar candidate, logical units are relative to the
                // compiler's package-root working directory, not user-provided
                // arbitrary labels or aliases for a different file.
                if input_id != package_root.join(source_unit) ||
                   !registry_id.starts_with(&package_root) {
                    return Err("source-ID path does not match physical package-root source or registry".into());
                }
                let object_id = canonical_destination(object)?;
                let map_id = canonical_destination(map)?;
                if input_id == registry_id || input_id == object_id || input_id == map_id
                    || registry_id == object_id || registry_id == map_id
                    || object_id == map_id {
                    return Err("source, registry, object and source map must be distinct files".into());
                }
                let source = bounded_read(input)?;
                let registry_json = bounded_read(registry)?;
                let (bytes, source_map) = oreslang_source_rs::compile_with_source_ids(
                    &source, source_unit, &registry_json)?;
                let map_bytes = serde_json::to_vec_pretty(&source_map).map_err(|e| e.to_string())?;
                fs::write(object, bytes).map_err(|e| e.to_string())?;
                fs::write(map, map_bytes).map_err(|e| e.to_string())?;
                return Ok(());
            }
        }

        let (mode, input, output, effects_output, budget) = match args.as_slice() {
            [_, flag, input] if flag == "--safety-ir" => ("safety", input.as_str(), None, None, None),
            [_, flag, input, units] if flag == "--safety-ir" => ("safety", input.as_str(), None, None,
                Some(units.parse::<u64>().map_err(|_| "invalid abstract compute budget")?)),
            [_, flag, units, input] if flag == "--flow" => ("flow", input.as_str(), None, None,
                Some(units.parse::<u64>().map_err(|_| "invalid abstract compute budget")?)),
            [_, flag, input] if flag == "--check" || flag == "--effects" || flag == "--static-facts" =>
                (flag.as_str(), input.as_str(), None, None, None),
            [_, flag, input, object, effects] if flag == "--compile-with-effects" =>
                (flag.as_str(), input.as_str(), Some(object.as_str()), Some(effects.as_str()), None),
            [_, flag, input] if flag == "--cfg-ir" => ("cfg", input.as_str(), None, None, None),
            [_, flag, input, units] if flag == "--cfg-ir" => ("cfg", input.as_str(), None, None,
                Some(units.parse::<u64>().map_err(|_| "invalid abstract compute budget")?)),
            [_, input, output] if !input.starts_with("--") =>
                ("object", input.as_str(), Some(output.as_str()), None, None),
            _ => return Err("usage: oresc --compile-with-source-ids <input.atto> <registry.json> <logical/source.atto> <out.o> <out.map.json> | oresc [--release|--strict-aot|--hard-rt] [--freestanding] [--checked-i64] --safety-ir <source.ores> [abstract-budget] | --flow <max-uninterrupted-units> <source.ores> | --check <source.ores> | --effects <source.ores> | --static-facts <source.ores> | --cfg-ir <source.ores> [abstract-budget] | --compile-with-effects <source.ores> <out.o> <out.json> | <source.ores> <out.o>".into()),
        };
        // Current analysis exporters model only linear scalar computations.
        // Refuse to issue falsely complete facts for checked overflow traps.
        if checked_i64 && mode != "object" {
            return Err("--checked-i64 cannot be combined with safety/effect/CFG/fact export: conditional overflow traps are not yet modeled".into());
        }
        let source_id = fs::canonicalize(input).map_err(|e| e.to_string())?;
        let object_id = if let Some(path) = output {
            let out = canonical_destination(path)?;
            if out == source_id {
                return Err("refusing to overwrite source with compiler output".into());
            }
            Some(out)
        } else {
            None
        };
        if let Some(path) = effects_output {
            let facts_id = canonical_destination(path)?;
            if facts_id == source_id || object_id.as_ref() == Some(&facts_id) {
                return Err("source, object and effect output files must be distinct".into());
            }
        }
        // Bound both ordinary and paired compiler invocations before tokenization.
        let mut source = String::new();
        fs::File::open(input)
            .map_err(|e| e.to_string())?
            .take(MAX_SOURCE_BYTES as u64 + 1)
            .read_to_string(&mut source)
            .map_err(|e| e.to_string())?;
        if source.len() > MAX_SOURCE_BYTES {
            return Err("source too large: exceeds 1 MiB admission budget".into());
        }
        let parsed = frontend::parse(&source).map_err(|e| e.to_string())?;
        let checked = mir::lower(parsed).map_err(|e| e.to_string())?;
        let _freestanding = freestanding;
        let checked = profile::apply(checked, profile)?;
        if mode == "--compile-with-effects" {
            let object = codegen::emit_object(&checked)?;
            let facts = effects::export_effect_ir_checked(&checked)?;
            let serialized = serde_json::to_vec(&facts).map_err(|e| e.to_string())?;
            // All compilation completes before writing either artifact. These
            // two file writes are not a transactional/atomic commit.
            fs::write(output.ok_or("missing native output")?, object).map_err(|e| e.to_string())?;
            fs::write(effects_output.ok_or("missing effect output")?, serialized)
                .map_err(|e| e.to_string())?;
        } else {
            match mode {
                "--check" => println!("OK: {} function(s)", checked.functions.len()),
                "safety" => println!(
                    "{}",
                    serde_json::to_string_pretty(&safety::export_safety_ir(&checked, budget))
                        .map_err(|e| e.to_string())?
                ),
                "flow" => println!(
                    "{}",
                    serde_json::to_string_pretty(&flow::export_scalar_flow(
                        &checked,
                        budget.ok_or("missing abstract work budget")?
                    )?)
                    .map_err(|e| e.to_string())?
                ),
                "cfg" => println!(
                    "{}",
                    serde_json::to_string_pretty(&cfg::export_cfg_v2(&checked, budget)?)
                        .map_err(|e| e.to_string())?
                ),
                "--static-facts" => println!(
                    "{}",
                    serde_json::to_string(&oreslang_source_rs::static_facts::export(
                        &checked, &source
                    )?)
                    .map_err(|e| e.to_string())?
                ),
                "--effects" => println!(
                    "{}",
                    serde_json::to_string_pretty(&effects::export_effect_ir_checked(&checked)?)
                        .map_err(|e| e.to_string())?
                ),
                _ => {
                    let bytes = if checked_i64 {
                        // Structural recursion/resource budget gate, not a
                        // checked-arithmetic effect or exception certificate.
                        let _ = effects::export_effect_ir_checked(&checked)?;
                        codegen::emit_object_checked_i64(&checked)?
                    } else {
                        effects::export_effect_ir_checked(&checked)?;
                        codegen::emit_object(&checked)?
                    };
                    fs::write(output.ok_or("missing output path")?, bytes)
                        .map_err(|e| e.to_string())?;
                }
            }
        }
        Ok(())
    })();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("oresc: {error}");
            ExitCode::from(1)
        }
    }
}
