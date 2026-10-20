use oreslang_source_rs::{
    cfg, codegen, effects, frontend, mir,
    profile::{self, BuildProfile},
    safety,
};
use std::{env, fs, io::Read, process::ExitCode};

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

        let (mode, input, output, budget) = match args.as_slice() {
            [_, flag, input] if flag == "--check" || flag == "--effects" => {
                (flag.as_str(), input.as_str(), None, None)
            }
            [_, flag, input] if flag == "--cfg-ir" => ("cfg", input.as_str(), None, None),
            [_, flag, input, units] if flag == "--cfg-ir" => (
                "cfg",
                input.as_str(),
                None,
                Some(
                    units
                        .parse::<u64>()
                        .map_err(|_| "invalid abstract compute budget")?,
                ),
            ),
            [_, flag, input] if flag == "--safety-ir" => {
                ("safety", input.as_str(), None, None)
            }
            [_, flag, input, units] if flag == "--safety-ir" => (
                "safety",
                input.as_str(),
                None,
                Some(
                    units
                        .parse::<u64>()
                        .map_err(|_| "invalid abstract compute budget")?,
                ),
            ),
            [_, input, output] => ("object", input.as_str(), Some(output.as_str()), None),
            _ => {
                return Err(
                    "usage: oresc [--release|--strict-aot|--hard-rt] [--freestanding] (--check <source.ores> | --effects <source.ores> | --cfg-ir <source.ores> [abstract-budget] | --safety-ir <source.ores> [abstract-budget] | <source.ores> <out.o>)"
                        .into(),
                )
            }
        };

        let mut source = String::new();
        fs::File::open(input)
            .map_err(|e| e.to_string())?
            .take(1_048_577)
            .read_to_string(&mut source)
            .map_err(|e| e.to_string())?;
        if source.len() > 1_048_576 {
            return Err("source too large".into());
        }

        let parsed = frontend::parse(&source).map_err(|e| e.to_string())?;
        let checked = mir::lower(parsed).map_err(|e| e.to_string())?;

        // The admitted scalar backend has no host calls, runtime reflection,
        // allocator, GC, or dynamic loading. Linking/platform providers remain
        // external; keep the flag explicit so later subsets cannot inherit an
        // accidental host-runtime assumption.
        let _freestanding = freestanding;
        let checked = profile::apply(checked, profile)?;

        match mode {
            "--check" => println!("OK: {} function(s)", checked.functions.len()),
            "cfg" => println!(
                "{}",
                serde_json::to_string_pretty(&cfg::export_cfg_v2(&checked, budget)?)
                    .map_err(|e| e.to_string())?
            ),
            "--effects" => println!(
                "{}",
                serde_json::to_string_pretty(&effects::export_effect_ir(&checked))
                    .map_err(|e| e.to_string())?
            ),
            "safety" => println!(
                "{}",
                serde_json::to_string_pretty(&safety::export_safety_ir(&checked, budget))
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
