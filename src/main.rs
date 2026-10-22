use oreslang_source_rs::{cfg, codegen, effects, flow, frontend, mir};
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
    let args: Vec<_> = env::args().collect();
    let result: Result<(), String> = (|| {
        let (mode, input, output, effects_output, budget) = match args.as_slice() {
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
            _ => return Err("usage: oresc --flow <max-uninterrupted-units> <source.ores> | --check <source.ores> | --effects <source.ores> | --static-facts <source.ores> | --cfg-ir <source.ores> [abstract-budget] | --compile-with-effects <source.ores> <out.o> <out.json> | <source.ores> <out.o>".into()),
        };
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
        if mode == "--compile-with-effects" {
            let (object, facts) = oreslang_source_rs::compile_with_effect_ir(&source)?;
            let serialized = serde_json::to_vec(&facts).map_err(|e| e.to_string())?;
            // All compilation completes before writing either artifact. These
            // two file writes are not a transactional/atomic commit.
            fs::write(output.ok_or("missing native output")?, object).map_err(|e| e.to_string())?;
            fs::write(effects_output.ok_or("missing effect output")?, serialized)
                .map_err(|e| e.to_string())?;
        } else {
            let parsed = frontend::parse(&source).map_err(|e| e.to_string())?;
            let checked = mir::lower(parsed).map_err(|e| e.to_string())?;
            match mode {
                "--check" => println!("OK: {} function(s)", checked.functions.len()),
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
                    effects::export_effect_ir_checked(&checked)?;
                    let bytes = codegen::emit_object(&checked)?;
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
