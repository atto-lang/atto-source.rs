//! Independent Rust Oreslang frontend, MIR and Cranelift backend.
pub mod codegen;
pub mod frontend;
pub mod effect_ir;
pub mod mir;

/// Compile supported source text to the host machine's relocatable object bytes.
pub fn compile(source: &str) -> Result<Vec<u8>, String> {
    let ast = frontend::parse(source).map_err(|e| e.to_string())?;
    let module = mir::lower(ast)?;
    codegen::emit_object(&module)
}
/// Compile object code and emit conservative scalar effect IR from *one*
/// parsed and validated MIR value. This avoids independent source parsing
/// silently diverging between object generation and effect export.
///
/// It does not authenticate this compiler binary or prove target WCET.
pub fn compile_with_effect_ir(source: &str) -> Result<(Vec<u8>, String), String> {
    let ast = frontend::parse(source).map_err(|e| e.to_string())?;
    let module = mir::lower(ast)?;
    // Fail closed before codegen if MIR contains an unaccounted-for effect.
    let effects = effect_ir::emit_effect_ir(&module)?;
    let object = codegen::emit_object(&module)?;
    Ok((object, effects))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compiles_native_object() {
        let bytes = compile("static fnc answer() -> int { return 40 + 2; }").unwrap();
        assert!(!bytes.is_empty());
        #[cfg(target_os = "linux")]
        assert!(bytes.starts_with(b"\x7fELF"));
    }
    #[test]
    fn emits_object_and_effects_from_same_validated_mir() {
        let source = include_str!("../examples/answer.ores");
        let (object, effects) = compile_with_effect_ir(source).unwrap();
        assert!(!object.is_empty());
        assert_eq!(effects, include_str!("../examples/scalar-effect-ir-v1.json").trim_end());
    }
    #[test]
    fn paired_compilation_denies_unmodeled_actor_or_io() {
        assert!(compile_with_effect_ir("define actor Foo as end").is_err());
        assert!(compile_with_effect_ir("static fnc f() as return readch(ch); end").is_err());
    }
    #[test]
    fn rejects_wrong_language_construct() {
        assert!(compile("define actor Foo as end").is_err());
    }
}
