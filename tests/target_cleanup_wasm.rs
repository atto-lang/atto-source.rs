//! Executed *Wasm instruction* cleanup witness tests. Source mtd stays
//! unsupported in the production compiler and receiver-instance preview.
use atto_compiler::target_cleanup_wasm::{
    audit_cleanup_wasm_witness,compile_cleanup_wasm_witness
};

const SOURCE:&str=include_str!("fixtures/cleanup/witness.atto");
const RELEASE:&[u8]=&[0x41,0,0x24,1,0x23,3,0x41,1,0x6a,0x24,3];

fn locations(haystack:&[u8],needle:&[u8])->Vec<usize>{
    haystack.windows(needle.len()).enumerate().filter_map(|(i,w)|
        (w==needle).then_some(i)).collect()
}

#[test]
fn verified_cleanup_wasm_contains_exactly_three_emitted_releases(){
    let bytes=compile_cleanup_wasm_witness(SOURCE).unwrap();
    assert_eq!(&bytes[..8],b"\0asm\x01\0\0\0");
    assert!(bytes.len()<4096);
    assert_eq!(bytes,compile_cleanup_wasm_witness(SOURCE).unwrap());
    audit_cleanup_wasm_witness(&bytes).unwrap();
    assert_eq!(locations(&bytes,RELEASE).len(),3,
        "cancel, checked overflow and normal exit must each unlock exactly once");
    for name in ["run_cleanup_witness","lease_active","release_count",
        "acquire_count","state_value","close_witness","panic_code"] {
        assert!(bytes.windows(name.len()).any(|w|w==name.as_bytes()),"missing {name}");
    }
}

#[test]
fn backend_auditor_rejects_mutated_release_opcode_on_emitted_module(){
    let bytes=compile_cleanup_wasm_witness(SOURCE).unwrap();
    let sites=locations(&bytes,RELEASE);
    assert_eq!(sites.len(),3);
    for site in sites {
        let mut forged=bytes.clone();
        forged[site+2]=0x23; // global.set -> global.get, release no longer works
        assert_eq!(audit_cleanup_wasm_witness(&forged).unwrap_err().code,"E_CLEANUP");
        let mut forged=bytes.clone();
        forged[site+6]=0x02; // release increments by 2, not exactly once
        assert_eq!(audit_cleanup_wasm_witness(&forged).unwrap_err().code,"E_CLEANUP");
    }
    let mut truncated=bytes;
    truncated.pop();
    assert!(audit_cleanup_wasm_witness(&truncated).is_err());
}

#[test]
fn production_compiler_never_uses_experimental_cleanup_witness_proof(){
    assert_eq!(atto_compiler::compile(SOURCE).unwrap_err().code,"E_UNSUPPORTED");
    assert_eq!(atto_compiler::target_scalar::compile_scalar_receiver_instance_preview(SOURCE)
        .unwrap_err().code,"E_UNSUPPORTED");
}

#[test]
fn cleanup_wasm_rejects_wrong_lease_shape_and_unsafe_callbacks(){
    for source in [
        "declare actor A; mtd (A self) get(int n): int { return n; }",
        "declare actor A; mtd (A self) get(int n): int { return n + 1; }",
        "declare actor Counter; define actor Counter as let int count=0; end
         mtd (Counter self) get(): int {
           return self.handle_state(critical-read |s| -> s.count + 1);
         }",
        "declare actor Counter; define actor Counter as let int count=0; end
         mtd (mut self: Counter) set(int delta): void {
           self.handle_state(critical-write |s| -> { s.count = s.count + delta; });
           self.handle_state(critical-write |t| -> { t.count = t.count + delta; });
         }",
        "declare actor Counter; define actor Counter as let int count=0; end
         mtd (mut self: Counter) set(int delta): void {
           self.handle_state(critical-write |s| -> { while true { s.count = delta; } });
         }",
    ]{
        assert!(compile_cleanup_wasm_witness(source).is_err(),
            "unsafe cleanup shape admitted: {source}");
    }
}
