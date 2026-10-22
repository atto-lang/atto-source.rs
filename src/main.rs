use oreslang_source_rs::{cfg, codegen, effects, frontend, mir};
use std::{env, fs, io::Read, process::ExitCode};

fn main() -> ExitCode {
    let args: Vec<_> = env::args().collect();
    let result: Result<(), String> = (|| {
        let (mode, input, output, budget) = match args.as_slice() {
            [_, flag, input] if flag == "--check" || flag == "--effects" || flag == "--static-facts" =>
                (flag.as_str(), input.as_str(), None, None),
            [_, flag, input] if flag == "--cfg-ir" =>
                ("cfg", input.as_str(), None, None),
            [_, flag, input, units] if flag == "--cfg-ir" =>
                ("cfg", input.as_str(), None, Some(units.parse::<u64>().map_err(|_| "invalid abstract compute budget")?)),
            [_, input, output] => ("object", input.as_str(), Some(output.as_str()), None),
            _ => return Err("usage: oresc --check <source.ores> | --effects <source.ores> | --static-facts <source.ores> | --cfg-ir <source.ores> [abstract-budget] | <source.ores> <out.o>".into()),
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
            "--static-facts" => println!(
                "{}",
                serde_json::to_string(&oreslang_source_rs::static_facts::export(
                    &checked, &source
                )?)
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
