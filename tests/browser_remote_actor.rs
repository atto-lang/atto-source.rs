use atto_compiler::compile;
fn source(body:&str)->String {
    format!("define actor Counter as
      receive(ActorMail<int> mail): void {{ return; }}
    end
    pub fnc main(): int {{ {body} return 0; }}")
}
#[test]
fn typed_remote_actor_send_lowers_to_an_executable_wasm_guest() {
    let src=source("val remote = remote_actor(Counter, 1); remote.send(40); remote.send(2);");
    let wasm=compile(&src).expect("remote actor send should compile");
    assert!(wasm.starts_with(b"\0asm\x01\0\0\0"));
}
#[test]
fn remote_actor_send_rejects_wrong_actor_type_and_unbound_or_dynamic_slot() {
    for expr in ["remote_actor(NoSuchActor, 1)",
        "remote_actor(Counter, 0)","remote_actor(Counter, 17)",
        "remote_actor(Counter, true)","remote_actor(Counter, -1)",
        "remote_actor(3, 1)"] {
        let src=source(&format!("val remote = {expr};"));
        assert!(compile(&src).is_err(),"accepted invalid source: {expr}");
    }
    let src=source("val remote = remote_actor(Counter, 1); remote.send(true);");
    assert!(compile(&src).is_err(),"remote ActorRef<int> must reject bool mail");
}
