//! Explicit host-only source-bound actor state MIR exporter.
//! Unlike attoc this NEVER emits a guest artifact.
use std::{env, fs, process};
fn main() {
    let args: Vec<_> = env::args_os().collect();
    let cfg_v1 = args.len() == 3 && args[1] == "--cfg-v1";
    if args.len() != 2 && !cfg_v1 {
        eprintln!("usage: atto-state-mir [--cfg-v1] <source.atto>");
        process::exit(2);
    }
    let path = &args[if cfg_v1 { 2 } else { 1 }];
    let data = match fs::read_to_string(path) {
        Ok(data) => data,
        Err(error) => { eprintln!("E_INPUT: {error}"); process::exit(2); }
    };
    if cfg_v1 {
        match atto_compiler::actor_state_cfg_v1::derive_bounded_diamond_cfg(&data) {
            Ok(cfg) => print!("{}", cfg.to_wire()),
            Err(diagnostic) => { eprintln!("{diagnostic}"); process::exit(1); }
        }
    } else {
        match atto_compiler::actor_state_mir_v0::derive_bounded_guard_mir(&data) {
            Ok(mir) => print!("{}", mir.to_wire()),
            Err(diagnostic) => { eprintln!("{diagnostic}"); process::exit(1); }
        }
    }
}
