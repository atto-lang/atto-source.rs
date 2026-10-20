//! Independent Rust Oreslang frontend, validated MIR and Cranelift backend.
pub mod codegen;
pub mod effects;
pub mod frontend;
pub mod mir;
pub mod ref_codegen;
pub mod ref_layout;

/// Compile the admitted scalar subset using the default managed-reference layout.
pub fn compile(source: &str) -> Result<Vec<u8>, String> {
    compile_with_ref_layout(source, ref_layout::ManagedRefLayout::default())
}

/// Compile while explicitly selecting the backend-neutral managed-reference layout.
/// The current scalar source subset has no Ref<T> operations yet, but the choice is
/// preserved in MIR for future actor/object lowering and static-analysis metadata.
pub fn compile_with_ref_layout(
    source: &str,
    managed_refs: ref_layout::ManagedRefLayout,
) -> Result<Vec<u8>, String> {
    let ast = frontend::parse(source).map_err(|e| e.to_string())?;
    let module = mir::lower_with_ref_layout(ast, managed_refs).map_err(|e| e.to_string())?;
    codegen::emit_object(&module)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compiles_real_native_object() {
        let result = compile(
            "pub fnc main(): int { return add(40,2); }\nfnc add(int a,int b): int { return a+b; }",
        );
        assert!(result.is_ok(), "{result:?}");
        let object = result.unwrap();
        #[cfg(target_os = "linux")]
        assert!(object.starts_with(b"\x7fELF"));
    }
    #[test]
    fn native_managed_ref_layout_compiles_same_scalar_semantics() {
        let result = compile_with_ref_layout(
            "pub fnc main(): int { return 42; }",
            ref_layout::ManagedRefLayout::native(),
        );
        assert!(result.is_ok(), "{result:?}");
    }

    #[test]
    fn rejects_unsupported_actor_syntax() {
        assert!(compile("define actor Foo as end").is_err());
    }
}
