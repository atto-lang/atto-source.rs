//! Contract-only tests: no claims of compiled HTTP/async guest execution.
use atto_compiler::{browser_http_abi_v1 as abi, compile};

#[test]
fn wire_names_types_and_numeric_codes_match_the_unshipped_js_host() {
    use abi::{Admission, CompletionTag, ErrorCode, ValueType};
    assert_eq!(abi::VERSION_SECTION, "atto.http.v1");
    assert_eq!(abi::VERSION_MARKER, [1]);
    assert_eq!(abi::IMPORT_MODULE, "atto_http_v1");
    assert_eq!(abi::IMPORT_REQUEST, "request");
    assert_eq!(abi::IMPORT_CANCEL, "cancel");
    assert_eq!(abi::EXPORT_RESERVE, "atto_http_v1_reserve");
    assert_eq!(abi::EXPORT_RESUME, "atto_http_v1_resume");
    assert_eq!(abi::REQUEST_PARAMS, [ValueType::I32; 3]);
    assert_eq!(abi::REQUEST_RESULTS, [ValueType::I32]);
    assert_eq!(abi::CANCEL_PARAMS, [ValueType::I32]);
    assert_eq!(abi::CANCEL_RESULTS, [ValueType::I32]);
    assert_eq!(abi::RESERVE_PARAMS, [ValueType::I32]);
    assert_eq!(abi::RESERVE_RESULTS, [ValueType::I32]);
    assert_eq!(abi::RESUME_PARAMS, [ValueType::I32; 5]);
    assert_eq!(Admission::Accepted as i32, 1);
    assert_eq!(Admission::BadRequest as i32, 2);
    assert_eq!(Admission::Closed as i32, 3);
    assert_eq!(Admission::Busy as i32, 4);
    assert_eq!(CompletionTag::Ok as i32, 0);
    assert_eq!(CompletionTag::Error as i32, 1);
    for (kind, code) in [
        (ErrorCode::Permission, 1), (ErrorCode::Aborted, 2),
        (ErrorCode::Timeout, 3), (ErrorCode::Quota, 4),
        (ErrorCode::Size, 5), (ErrorCode::Redirect, 6),
        (ErrorCode::Opaque, 7), (ErrorCode::Body, 8),
        (ErrorCode::Closed, 9), (ErrorCode::Transport, 255),
    ] {
        assert_eq!(kind as i32, code);
    }
    assert_eq!(abi::DEFAULT_RESPONSE_BYTES, 1024 * 1024);
    assert_eq!(abi::MAX_TIMEOUT_MS, 120_000);
    assert_eq!(abi::DEFAULT_TIMEOUT_MS, 30_000);
    assert_eq!(abi::MAX_URL_BYTES, 2048);
}

#[test]
fn request_ids_are_scoped_to_actor_incarnation_and_generation() {
    use abi::{PendingRequests, Ticket, TicketError};
    assert_eq!(PendingRequests::new(0, 1).unwrap_err(), TicketError::InvalidConfiguration);
    assert_eq!(PendingRequests::new(1, 0).unwrap_err(), TicketError::InvalidConfiguration);
    assert_eq!(
        PendingRequests::new(1, abi::MAX_REQUESTS + 1).unwrap_err(),
        TicketError::InvalidConfiguration
    );
    let mut table = PendingRequests::new(22, 2).unwrap();
    assert_eq!(table.admit(0).unwrap_err(), TicketError::InvalidId);
    assert_eq!(table.admit(-1).unwrap_err(), TicketError::InvalidId);
    let first = table.admit(7).unwrap();
    let second = table.admit(8).unwrap();
    assert_eq!(table.pending(), 2);
    assert_eq!(table.admit(7).unwrap_err(), TicketError::Duplicate);
    assert_eq!(table.admit(9).unwrap_err(), TicketError::Busy);
    assert_eq!(first.id.value(), 7);
    assert_eq!(
        table.settle(Ticket { incarnation: 21, ..first }).unwrap_err(),
        TicketError::Stale
    );
    table.settle(first).unwrap();
    assert_eq!(table.settle(first).unwrap_err(), TicketError::Stale);
    assert_eq!(table.pending(), 1);
    assert_eq!(table.revoke(), 1);
    assert_eq!(table.pending(), 0);
    assert_eq!(table.admit(7).unwrap_err(), TicketError::Closed);
    assert_eq!(table.settle(second).unwrap_err(), TicketError::Closed);
}

#[test]
fn bounded_guest_owned_response_memory_and_error_envelopes() {
    use abi::{validate_completion, CompletionTag, TicketError};
    assert!(validate_completion(CompletionTag::Ok, 200, 1024, 3, 65536, 1024).is_ok());
    assert!(validate_completion(CompletionTag::Ok, 599, 65536, 0, 65536, 1024).is_ok());
    assert!(validate_completion(CompletionTag::Error, 2, 0, 0, 65536, 1024).is_ok());
    for (tag, code, ptr, len) in [
        (CompletionTag::Ok, 199, 0, 0),
        (CompletionTag::Ok, 600, 0, 0),
        (CompletionTag::Ok, 200, 65535, 3),
        (CompletionTag::Ok, 200, u32::MAX, 1),
        (CompletionTag::Ok, 200, 100, 1025),
        (CompletionTag::Error, 1, 1, 1),
        (CompletionTag::Error, 0, 0, 0),
    ] {
        assert_eq!(
            validate_completion(tag, code, ptr, len, 65536, 1024),
            Err(TicketError::InvalidConfiguration),
            "unexpected accepted envelope: {tag:?} {code} {ptr} {len}"
        );
    }
}

#[test]
fn async_fetch_preview_stays_non_executable_until_real_continuation_lowering() {
    for source in [
        "pub fnc main(): int { return await fetch(\"https://example.com\"); }",
        "pub async fnc main(): int { return 42; }",
    ] {
        assert!(
            compile(source).is_err(),
            "async source must not emit ordinary synchronous guest Wasm"
        );
    }
    let ordinary = compile("pub fnc main(): int { return 42; }").unwrap();
    assert!(ordinary.starts_with(b"\0asm"));
    assert!(
        !ordinary.windows(abi::IMPORT_MODULE.len()).any(|x| x == abi::IMPORT_MODULE.as_bytes()),
        "ordinary code must not import experimental HTTP capability"
    );
}
