use oreslang_source_rs::{codegen, effects, flow, frontend, mir};
use std::{env, fs, process::ExitCode};

fn main() -> ExitCode {
    let args: Vec<_> = env::args().collect();
    let result: Result<(), String> = (|| {
        let (mode, input, output) = match args.as_slice() {
            [_, flag, budget, input] if flag == "--flow" =>
                (flag.as_str(), input.as_str(), Some(budget.as_str())),
            [_, flag, input] if flag == "--check" || flag == "--effects" =>
                (flag.as_str(), input.as_str(), None),
            [_, input, output] => ("object", input.as_str(), Some(output.as_str())),
            _ => return Err("usage: oresc --check <source.ores> | --effects <source.ores> | --flow <max-uninterrupted-units> <source.ores> | <source.ores> <out.o>".into()),
        };
        let source = fs::read_to_string(input).map_err(|e| e.to_string())?;
        let parsed = frontend::parse(&source).map_err(|e| e.to_string())?;
        let checked = mir::lower(parsed).map_err(|e| e.to_string())?;
        match mode {
            "--flow" => {
                let budget = output.ok_or("missing abstract operation budget")?
                    .parse::<u64>().map_err(|e| e.to_string())?;
                let envelope = flow::export_scalar_flow(&checked, budget)?;
                println!("{}", serde_json::to_string_pretty(&envelope)
                    .map_err(|e| e.to_string())?);
            }
            "--check" => println!("OK: {} function(s)", checked.functions.len()),
            "--effects" => println!("{}", serde_json::to_string_pretty(
                &effects::export_effect_ir(&checked)).map_err(|e| e.to_string())?),
            _ => {
                let bytes = codegen::emit_object(&checked)?;
                fs::write(output.ok_or("missing output path")?, bytes).map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    })();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => { eprintln!("oresc: {error}"); ExitCode::from(1) }
    }
}
