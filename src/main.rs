use oreslang_source_rs::{codegen, effects, frontend, mir, profile::{self, BuildProfile}, safety};
use std::{env, fs, process::ExitCode};

fn main() -> ExitCode {
    let raw_args: Vec<String> = env::args().collect();
    let result: Result<(), String> = (|| {
        let mut profile = BuildProfile::Dev;
        let mut profile_seen = false;
        let mut freestanding = false;
        let mut args = Vec::with_capacity(raw_args.len());
        args.push(raw_args[0].clone());
        for arg in raw_args.iter().skip(1) {
            if let Some(selected) = BuildProfile::parse_flag(arg) {
                if profile_seen {
                    return Err("--release, --strict-aot and --hard-rt are mutually exclusive".into());
                }
                profile = selected;
                profile_seen = true;
            } else if arg == "--freestanding" {
                if freestanding {
                    return Err("duplicate --freestanding".into());
                }
                freestanding = true;
            } else {
                args.push(arg.clone());
            }
        }
        let (mode, input, output, policy_budget) = match args.as_slice() {
            [_, flag, input] if flag == "--check" || flag == "--effects" =>
                (flag.as_str(), input.as_str(), None, None),
            [_, flag, input] if flag == "--safety-ir" =>
                (flag.as_str(), input.as_str(), None, None),
            [_, flag, input, units] if flag == "--safety-ir" =>
                ("--safety-ir", input.as_str(), None, Some(units.parse::<u64>().map_err(|_| "invalid abstract compute budget")?)),
            [_, input, output] =>
                ("object", input.as_str(), Some(output.as_str()), None),
            _ => return Err("usage: oresc [--release|--strict-aot|--hard-rt] [--freestanding] (--check <source.ores> | --effects <source.ores> | --safety-ir <source.ores> [abstract-unit-budget] | <source.ores> <out.o>)".into()),
        };
        let source = fs::read_to_string(input).map_err(|e| e.to_string())?;
        let parsed = frontend::parse(&source).map_err(|e| e.to_string())?;
        let checked = mir::lower(parsed).map_err(|e| e.to_string())?;
        // The admitted scalar backend has no host calls, runtime reflection,
        // allocator, GC, or dynamic loading, so its object is freestanding at
        // this stage. Linking/platform providers remain an external concern.
        let _freestanding = freestanding;
        let checked = profile::apply(checked, profile)?;
        match mode {
            "--check" => println!("OK: {} function(s)", checked.functions.len()),
            "--effects" => println!(
                "{}",
                serde_json::to_string_pretty(&effects::export_effect_ir(&checked))
                    .map_err(|e| e.to_string())?
            ),
            "--safety-ir" => println!(
                "{}",
                serde_json::to_string_pretty(&safety::export_safety_ir(&checked, policy_budget))
                    .map_err(|e| e.to_string())?
            ),
            _ => {
                let bytes = codegen::emit_object(&checked)?;
                fs::write(output.ok_or("missing output path")?, bytes)
                    .map_err(|e| e.to_string())?;
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
