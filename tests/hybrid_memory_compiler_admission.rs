//! Source-compiler memory admission, separate from standalone Ref32 host model.
//! This test must be run by the real Rust/Wasm compiler at the exact commit.
//! It does not establish general actor heap semantics or zero guest GC.
use atto_compiler::compile;

const SCALAR_ACTOR: &str = r#"
define actor Counter as
  let int total = 0;
  receive(ActorMail<int> mail): void {
    self.total = self.total + mail.value;
    return;
  }
end
pub fnc main(): int {
  val counter = spawn Counter();
  counter.send(1);
  return 0;
}
"#;

// Same object/ownership *shape* as the .NET opt-in generated-NativeAOT target.
// The Rust/Wasm actor scalar lane must not silently produce a fake bytecode
// success for this still-unverified nullable class-reference graph.
const LINKED_NODE_ACTOR: &str = include_str!("fixtures/hybrid_memory_node_graph.atto");

#[test]
fn existing_scalar_actor_continues_to_compile_as_real_wasm() {
    let wasm = compile(SCALAR_ACTOR).expect("previously admitted scalar actor guest");
    assert!(wasm.starts_with(b"\0asm\x01\0\0\0"), "real WebAssembly artifact");
    // The existing actor_memory_contract suite separately proves its bounded
    // one-page actor-private memory export. Do not conflate that with Ref32.
}

#[test]
fn nullable_linked_actor_graph_requires_real_lowering_before_any_wasm_artifact() {
    let error = compile(LINKED_NODE_ACTOR)
        .expect_err("unverified Ref32 graph must not silently emit executable Wasm");
    assert!(error.code.starts_with("E_"), "typed diagnostic required: {error}");
    // This rejection is a security/support gate, not proof of guest ownership.
}
