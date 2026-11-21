//! Explicit host-only source-bound actor state MIR exporter.
//! Unlike attoc this NEVER emits a guest artifact.
use std::{env, fs, process};
fn main() {
    let args: Vec<_> = env::args_os().collect();
    if args.len() != 2 {
        eprintln!("usage: atto-state-mir <source.atto>");
        process::exit(2);
    }
    let path = &args[1];
    let data = match fs::read_to_string(path) {
        Ok(data) => data,
        Err(error) => { eprintln!("E_INPUT: {error}"); process::exit(2); }
    };
    match atto_compiler::actor_state_mir_v0::derive_bounded_guard_mir(&data) {
        Ok(mir) => print!("{}", mir.to_wire()),
        Err(diagnostic) => { eprintln!("{diagnostic}"); process::exit(1); }
    }
}
