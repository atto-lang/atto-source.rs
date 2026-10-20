use oreslang_source_rs::{codegen, effects, frontend, mir};
use std::{env, fs, io::Read, process::ExitCode};

fn main() -> ExitCode {
    let args: Vec<_> = env::args().collect();
    let result: Result<(), String> = (|| {
        let (mode, input, output) = match args.as_slice() {
            [_, flag, input] if flag == "--check" || flag == "--effects" =>
                (flag.as_str(), input.as_str(), None),
            [_, input, output] => ("object", input.as_str(), Some(output.as_str())),
            _ => return Err("usage: oresc --check <source.ores> | --effects <source.ores> | <source.ores> <out.o>".into()),
        };
        let file = fs::File::open(input).map_err(|e| e.to_string())?;
        let mut source = String::new();
        file.take(frontend::MAX_SOURCE_BYTES as u64 + 1)
            .read_to_string(&mut source).map_err(|e| e.to_string())?;
        if source.len() > frontend::MAX_SOURCE_BYTES {
            return Err("source exceeds 1 MiB budget".into());
        }
        let parsed = frontend::parse(&source).map_err(|e| e.to_string())?;
        let checked = mir::lower(parsed).map_err(|e| e.to_string())?;
        match mode {
            "--check" => println!("OK: {} function(s)", checked.functions.len()),
            "--effects" => println!("{}", serde_json::to_string_pretty(
                &effects::export_effect_ir(&checked)?).map_err(|e| e.to_string())?),
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
