use std::{env, fs, io::Read, path::Path, process::ExitCode};

/// Prevent diagnostics/effect output from destroying compiler input or
/// colliding with the native object, including existing symlinks.
fn canonical_destination(path: &str) -> Result<std::path::PathBuf, String> {
    let candidate = Path::new(path);
    // symlink_metadata observes dangling symlinks too; never let a broken
    // output symlink evade identity checks by reporting exists() == false.
    if fs::symlink_metadata(candidate).is_ok() {
        return fs::canonicalize(candidate).map_err(|e| e.to_string());
    }
    let parent = candidate.parent().filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let dir = fs::canonicalize(parent).map_err(|e| e.to_string())?;
    let name = candidate.file_name().ok_or("output requires a filename")?;
    Ok(dir.join(name))
}

fn main() -> ExitCode {
    let args: Vec<_> = env::args().collect();
    let export_effect_ir = args.len() == 4 && args[1] == "--emit-effect-ir";
    let paired = args.len() == 5 && args[1] == "--compile-with-effects";
    if (args.len() != 3 && !export_effect_ir && !paired)
        || (args.len() == 3 && args[1].starts_with("--")) {
        eprintln!("usage: oresc <source.ores> <out.o> | oresc --emit-effect-ir <source.ores> <out.json> | oresc --compile-with-effects <source.ores> <out.o> <out.json>");
        return ExitCode::from(2);
    }
    let result: Result<(), String> = (|| {
        let source_path = if export_effect_ir || paired { &args[2] } else { &args[1] };
        let output_path = if export_effect_ir || paired { &args[3] } else { &args[2] };
        let source_id = fs::canonicalize(source_path).map_err(|e| e.to_string())?;
        let output_id = canonical_destination(output_path)?;
        if source_id == output_id {
            return Err("refusing to overwrite compiler source with output".into());
        }
        if paired {
            let effect_id = canonical_destination(&args[4])?;
            if effect_id == output_id || effect_id == source_id {
                return Err("object, effects and source must be distinct files".into());
            }
        }
        let file = fs::File::open(source_path).map_err(|e| e.to_string())?;
        let mut source = String::new();
        file.take(oreslang_source_rs::frontend::MAX_SOURCE_BYTES as u64 + 1)
            .read_to_string(&mut source).map_err(|e| e.to_string())?;
        if source.len() > oreslang_source_rs::frontend::MAX_SOURCE_BYTES {
            return Err("source exceeds byte budget".into());
        }
        if paired {
            // Both outputs are constructed from the same in-memory MIR before
            // either file is written. A two-file filesystem write is NOT atomic.
            let (object, json) = oreslang_source_rs::compile_with_effect_ir(&source)?;
            fs::write(output_path, object).map_err(|e| e.to_string())?;
            fs::write(&args[4], json).map_err(|e| e.to_string())?;
        } else if export_effect_ir {
            let json = oreslang_source_rs::effect_ir::source_to_effect_ir(&source)?;
            fs::write(output_path, json).map_err(|e| e.to_string())?;
        } else {
            let object = oreslang_source_rs::compile(&source)?;
            fs::write(output_path, object).map_err(|e| e.to_string())?;
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
