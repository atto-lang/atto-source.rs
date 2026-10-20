//! Independent Rust Oreslang frontend, validated MIR and Cranelift backend.
pub mod codegen;
pub mod effects;
pub mod frontend;
pub mod mir;
pub mod ref_layout;

/// Compile the admitted scalar subset into host relocatable object bytes.
pub fn compile(source: &str) -> Result<Vec<u8>, String> {
    let ast = frontend::parse(source).map_err(|e| e.to_string())?;
    let module = mir::lower(ast).map_err(|e| e.to_string())?;
    codegen::emit_object(&module)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compiles_real_native_object() {
        let result = compile("pub fnc main(): int { return add(40,2); }\nfnc add(int a,int b): int { return a+b; }");
        assert!(result.is_ok(), "{result:?}");
        let object = result.unwrap();
        #[cfg(target_os = "linux")]
        assert!(object.starts_with(b"\x7fELF"));
    }
    #[test]
    fn rejects_unsupported_actor_syntax() {
        assert!(compile("define actor Foo as end").is_err());
    }
}
