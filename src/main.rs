use std::{env, fs, io::Read, process::ExitCode};
fn main() -> ExitCode {
    let args: Vec<_> = env::args().collect();
    let export_effect_ir = args.len() == 4 && args[1] == "--emit-effect-ir";
    if args.len() != 3 && !export_effect_ir {
        eprintln!("usage: oresc <source.ores> <out.o> | oresc --emit-effect-ir <source.ores> <out.json>");
        return ExitCode::from(2);
    }
    let result: Result<(), String> = (|| {
        let source_path = if export_effect_ir { &args[2] } else { &args[1] };
        let output_path = if export_effect_ir { &args[3] } else { &args[2] };
        let file = fs::File::open(source_path).map_err(|e| e.to_string())?;
        let mut source = String::new();
        file.take(oreslang_source_rs::frontend::MAX_SOURCE_BYTES as u64 + 1)
            .read_to_string(&mut source).map_err(|e| e.to_string())?;
        if source.len() > oreslang_source_rs::frontend::MAX_SOURCE_BYTES {
            return Err("source exceeds byte budget".into());
        }
        if export_effect_ir {
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
