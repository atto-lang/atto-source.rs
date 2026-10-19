use std::{env, fs, io::Read, process::ExitCode};
fn main() -> ExitCode {
    let args: Vec<_> = env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: oresc <source.ores> <out.o>");
        return ExitCode::from(2);
    }
    let result: Result<(), String> = (|| {
        let file = fs::File::open(&args[1]).map_err(|e| e.to_string())?;
        let mut source = String::new();
        file.take(oreslang_source_rs::frontend::MAX_SOURCE_BYTES as u64 + 1)
            .read_to_string(&mut source).map_err(|e| e.to_string())?;
        if source.len() > oreslang_source_rs::frontend::MAX_SOURCE_BYTES {
            return Err("source exceeds byte budget".into());
        }
        let object = oreslang_source_rs::compile(&source)?;
        fs::write(&args[2], object).map_err(|e| e.to_string())?;
        Ok(())
    })();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => { eprintln!("oresc: {error}"); ExitCode::from(1) }
    }
}
