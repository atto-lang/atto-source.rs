//! Independent Rust Oreslang frontend, validated MIR and Cranelift backend.
pub mod cfg;
pub mod codegen;
pub mod effects;
pub mod flow;
pub mod frontend;
pub mod mir;
pub mod profile;
pub mod safety;
pub mod static_facts;
pub mod ref_codegen;
pub mod ref_layout;

/// Compile the current scalar grammar with opt-in signed i64 overflow traps.
/// No safety/effect certificate is emitted: those IR schemas cannot yet
/// express the new conditional integer-overflow failure edges.
pub fn compile_checked_i64(source: &str) -> Result<Vec<u8>, String> {
    let ast = frontend::parse(source).map_err(|e| e.to_string())?;
    let module = mir::lower(ast).map_err(|e| e.to_string())?;
    // Reuse the bounded traversal guard only; its v1 facts do not model
    // checked overflow edges and must not be published as a certificate.
    let _ = effects::export_effect_ir_checked(&module)?;
    codegen::emit_object_checked_i64(&module)
}

/// Compile the admitted scalar subset using the default managed-reference layout.
pub fn compile(source: &str) -> Result<Vec<u8>, String> {
    compile_with_ref_layout(source, ref_layout::ManagedRefLayout::default())
}

/// Compile while explicitly selecting the backend-neutral managed-reference layout.
/// Oreslang source does not expose a Ref<T>/pointer operation surface; this choice is
/// preserved in MIR for future actor/object lowering and static-analysis metadata.
pub fn compile_with_ref_layout(
    source: &str,
    managed_refs: ref_layout::ManagedRefLayout,
) -> Result<Vec<u8>, String> {
    let ast = frontend::parse(source).map_err(|e| e.to_string())?;
    let module = mir::lower_with_ref_layout(ast, managed_refs).map_err(|e| e.to_string())?;
    // Preserve mainline admission before ordinary code generation.
    effects::export_effect_ir_checked(&module)?;
    codegen::emit_object(&module)
}
/// Generate the object and its conservative effect-IR model from precisely
/// one parsed + type-checked Oreslang MIR. This is model conformance evidence,
/// not authenticated compiler provenance or physical hard-RT certification.
pub fn compile_with_effect_ir(source: &str) -> Result<(Vec<u8>, serde_json::Value), String> {
    let ast = frontend::parse(source).map_err(|e| e.to_string())?;
    let module = mir::lower(ast).map_err(|e| e.to_string())?;
    let effects = effects::export_effect_ir_checked(&module)?;
    let object = codegen::emit_object(&module)?;
    Ok((object, effects))
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
        assert!(!object.is_empty());
        #[cfg(target_os = "linux")]
        assert!(object.starts_with(b"\x7fELF"));
    }
    #[test]
    fn paired_mode_uses_current_scalar_mir_and_call_graph() {
        let input =
            "pub fnc main(): int { return add(40,2); }\nfnc add(int a,int b): int { return a+b; }";
        let (object, effects) = compile_with_effect_ir(input).unwrap();
        assert!(!object.is_empty());
        assert_eq!(effects["functions"][0]["operations"][2]["kind"], "call");
        assert_eq!(effects["functions"][0]["operations"][2]["target"], "add");
        assert_eq!(
            effects["functions"][1]["operations"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
    }
    #[test]
    fn paired_mode_rejects_unverified_actor_syntax() {
        assert!(compile_with_effect_ir("define actor Foo as end").is_err());
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
