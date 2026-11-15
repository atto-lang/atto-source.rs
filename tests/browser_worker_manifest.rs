use atto_compiler::browser_worker::{compile_browser_tenant, GUEST_ABI, TENANT_PROTOCOL};

#[test]
fn rust_compiler_packages_actor_names_for_browser_tenant() {
    let source = r#"
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
    let artifact = compile_browser_tenant(source).expect("actor fixture should compile");
    assert_eq!(artifact.actor_names, vec!["Counter".to_string()]);
    assert_eq!(artifact.wire_protocol, TENANT_PROTOCOL);
    assert_eq!(artifact.guest_abi, GUEST_ABI);
    assert!(artifact.wasm.starts_with(b"\0asm\x01\0\0\0"));
}

#[test]
fn unsupported_frontend_syntax_still_fails_without_artifact() {
    assert!(compile_browser_tenant(
        "pub fnc main(): int { return unsupported_identifier; }"
    ).is_err());
}
