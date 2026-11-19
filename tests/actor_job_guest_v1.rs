//! Executable *compiler* contract for provisional Job<int> guest ABI.
//! These Rust tests compile .atto source to real Wasm, but DO NOT prove
//! runtime ACK authentication or durable completion. Those require the
//! companion tenant host's guest-Wasm integration tests.

use atto_compiler::compile;

const GUEST: &str = r#"
define serial actor Worker as
    receive(Job<int> job): void {
        stdio.println(job.job_id);
        stdio.println(job.correlation_id);
        stdio.println(job.attempt);
        stdio.println(job.payload);
        job.ack();
        return;
    }
end
pub fnc main(): int { return 0; }
"#;

fn code(source: &str) -> String {
    compile(source).expect_err("guest must fail closed").code.into()
}

#[test]
fn job_receive_compiles_to_real_deterministic_wasm_with_version_marker() {
    let artifact=compile(GUEST).expect("typed job ingress should compile");
    assert!(artifact.starts_with(b"\0asm\x01\0\0\0"));
    assert_eq!(artifact,compile(GUEST).unwrap());
    assert!(artifact.windows(b"atto.job.abi".len())
        .any(|w|w==b"atto.job.abi"));
    assert!(artifact.windows(b"actor_receive_job".len())
        .any(|w|w==b"actor_receive_job"));
    assert!(artifact.windows(b"actor_job_ack".len())
        .any(|w|w==b"actor_job_ack"));
}

#[test]
fn legacy_plain_and_actor_mail_receives_keep_old_abi() {
    for kind in ["ActorMail<int> job","payload: int"] {
        let source=format!("define serial actor Worker as\nreceive({kind}): void {{ return; }}\nend\npub fnc main(): int {{ return 0; }}");
        let artifact=compile(&source).unwrap();
        assert!(!artifact.windows(b"atto.job.abi".len()).any(|w|w==b"atto.job.abi"));
        assert!(!artifact.windows(b"actor_receive_job".len()).any(|w|w==b"actor_receive_job"));
    }
}

#[test]
fn job_bool_and_other_untransportable_types_are_rejected() {
    let source=GUEST.replace("Job<int>","Job<bool>");
    assert_eq!(code(&source),"E_UNSUPPORTED_TYPE");
}

#[test]
fn job_ack_cannot_be_called_with_extra_parameters() {
    let source=GUEST.replace("job.ack();","job.ack(1);");
    assert_eq!(code(&source),"E_ARITY");
}

#[test]
fn unknown_job_fields_fail_closed() {
    let source=GUEST.replace("job.payload","job.owner");
    assert_eq!(code(&source),"E_UNBOUND");
}

#[test]
fn ordinary_actor_mail_cannot_forge_job_identity() {
    let source=GUEST.replace("Job<int>","ActorMail<int>");
    assert_eq!(code(&source),"E_UNBOUND");
}

#[test]
fn job_envelope_is_only_an_actor_receive_parameter() {
    let source="pub fnc main(): int { let Job<int> forged; return 0; }";
    assert!(compile(source).is_err());
}
