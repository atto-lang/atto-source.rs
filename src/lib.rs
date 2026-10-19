//! Independent Rust Oreslang frontend, MIR and Cranelift backend.
pub mod codegen;
pub mod frontend;
pub mod mir;

/// Compile supported source text to the host machine's relocatable object bytes.
pub fn compile(source: &str) -> Result<Vec<u8>, String> {
    let ast = frontend::parse(source).map_err(|e| e.to_string())?;
    let module = mir::lower(ast)?;
    codegen::emit_object(&module)
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
    fn rejects_wrong_language_construct() {
        assert!(compile("define actor Foo as end").is_err());
    }
}
