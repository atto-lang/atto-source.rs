//! Source-level Channel<int> v1, restricted guest-only nonparking lifecycle.
use atto_compiler::compile;

const MAIN: &str = "pub fnc main(): int { return 0; }";
fn src(body:&str)->String {
    format!("pub susp fnc read_channel_guest_v1(Channel<int> seed): Result<Option<int>,IoError> {{ {body} }} {MAIN}")
}
fn has(wasm:&[u8], name:&str)->bool {wasm.windows(name.len()).any(|w|w==name.as_bytes())}

#[test]
fn checked_open_try_send_close_release_lower_into_one_host_abi() {
    let body=r#"
        val opened = Channel<int>.bounded(2);
        match opened over
            on Ok(ch) -> {
                val status = ch.try_send(41);
                val received = readch ch;
                val closed = ch.close();
                val released = ch.release();
                return received;
            }
            on Err(error) -> { return Err(error); }
        end
    "#;
    let wasm=compile(&src(body)).expect("compiled bounded channel with checked lifecycle");
    assert!(wasm.starts_with(b"\0asm\x01\0\0\0"));
    for key in ["atto.channel_int","atto.async",
        "channel_int_open_v1","channel_int_send_v1",
        "channel_int_close_v1","channel_int_release_v1",
        "channel_int_nb_read_v1","channel_int_await_read_v1"] {
        assert!(has(&wasm,key),"missing checked channel guest import/marker {key}");
    }
}

#[test]
fn no_implicit_unwrap_or_wrong_payload_is_admitted() {
    for (body,code) in [
        ("val c: Channel<int> = Channel<int>.bounded(1); return readch c;", "E_TYPE"),
        ("val c=Channel<int>.bounded(\"one\"); return Err(0);","E_TYPE"),
        ("val c=Channel<int>.bounded(); return Err(0);","E_ARITY"),
        ("val c=Channel<int>.bounded(1,2); return Err(0);","E_ARITY"),
        ("val c=Channel<int>.bounded(1); return 42;","E_TYPE"),
    ] {
        let p=src(body);
        let d=compile(&p).expect_err("invalid constructor never returns guest artifact");
        assert_eq!(d.code,code,"{body}: {d}");
    }
}

#[test]
fn method_signatures_are_statically_checked() {
    let body=r#"match Channel<int>.bounded(1) over
       on Ok(ch) -> { ch.try_send("not an int"); return Err(0); }
       on Err(error) -> { return Err(error); }
       end"#;
    let err=compile(&src(body)).expect_err("no untyped channel write");
    assert_eq!(err.code,"E_TYPE","{err}");
}
