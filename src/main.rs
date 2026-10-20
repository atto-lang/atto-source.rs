use oreslang_source_rs::{codegen, effects, frontend, mir, safety};
use std::{env, fs, process::ExitCode};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    let result: Result<(), String> = (|| {
        let (mode, input, output, policy_budget) = match args.as_slice() {
            [_, flag, input] if flag == "--check" || flag == "--effects" =>
                (flag.as_str(), input.as_str(), None, None),
            [_, flag, input] if flag == "--safety-ir" =>
                (flag.as_str(), input.as_str(), None, None),
            [_, flag, input, units] if flag == "--safety-ir" =>
                ("--safety-ir", input.as_str(), None, Some(units.parse::<u64>().map_err(|_| "invalid abstract compute budget")?)),
            [_, input, output] =>
                ("object", input.as_str(), Some(output.as_str()), None),
            _ => return Err("usage: oresc --check <source.ores> | --effects <source.ores> | --safety-ir <source.ores> [abstract-unit-budget] | <source.ores> <out.o>".into()),
        };
        let source = fs::read_to_string(input).map_err(|e| e.to_string())?;
        let parsed = frontend::parse(&source).map_err(|e| e.to_string())?;
        let checked = mir::lower(parsed).map_err(|e| e.to_string())?;
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
