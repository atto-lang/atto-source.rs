use std::{env, fs, process::ExitCode};
fn main() -> ExitCode {
    let args: Vec<_> = env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: oresc <source.ores> <out.o>");
        return ExitCode::from(2);
    }
    let result: Result<(), String> = (|| {
        let source = fs::read_to_string(&args[1]).map_err(|e| e.to_string())?;
        let object = oreslang_source_rs::compile(&source)?;
        fs::write(&args[2], object).map_err(|e| e.to_string())?;
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
