use atto_compiler::browser_worker::{
    compile_browser_tenant,RemoteReceiptCode,RemoteReceiptHeader,
    RemoteReceiptOutcome,SEND_RECEIPT_PROTOCOL,
};
use atto_compiler::compile;

#[test]
fn typed_receipt_schema_matches_browser_and_wasm_contract() {
    assert_eq!(SEND_RECEIPT_PROTOCOL,"atto.browser.send-receipt.v1");
    assert_eq!(RemoteReceiptOutcome::Processed.wire(),"processed");
    assert_eq!(RemoteReceiptOutcome::Rejected.wire(),"rejected");
    assert_eq!(RemoteReceiptOutcome::Unknown.wire(),"unknown");
    let errors=[
        RemoteReceiptCode::RouteStale,
        RemoteReceiptCode::RouteBackpressure,
        RemoteReceiptCode::RouteTimeout,
        RemoteReceiptCode::RouteUnavailable,
        RemoteReceiptCode::RouteCrash,
        RemoteReceiptCode::RouteRevoked,
        RemoteReceiptCode::RouteReceiver,
        RemoteReceiptCode::RouteProtocol,
        RemoteReceiptCode::RouteUnknown,
    ];
    assert_eq!(errors.map(|x|x.wire()),[
        "E_ROUTE_STALE","E_ROUTE_BACKPRESSURE","E_ROUTE_TIMEOUT",
        "E_ROUTE_UNAVAILABLE","E_ROUTE_CRASH","E_ROUTE_REVOKED",
        "E_ROUTE_RECEIVER","E_ROUTE_PROTOCOL","E_ROUTE_UNKNOWN",
    ]);
    let head=RemoteReceiptHeader{
        receipt_id:1,route_id:2,source_generation:3,target_generation:4,
        actor_id:1,outcome:RemoteReceiptOutcome::Processed,code:RemoteReceiptCode::Ok
    };
    assert!(head.is_valid());
    assert!(!RemoteReceiptHeader{receipt_id:0,..head}.is_valid());
    assert!(!RemoteReceiptHeader{outcome:RemoteReceiptOutcome::Unknown,..head}.is_valid());
    assert!(RemoteReceiptHeader{outcome:RemoteReceiptOutcome::Unknown,
        code:RemoteReceiptCode::RouteCrash,..head}.is_valid());
    assert!(RemoteReceiptHeader{outcome:RemoteReceiptOutcome::Rejected,
        code:RemoteReceiptCode::RouteStale,..head}.is_valid());
}
#[test]
fn manifest_advertises_receipt_version_without_new_guest_import() {
    let artifact=compile_browser_tenant("pub fnc main(): int { return 42; }").unwrap();
    assert_eq!(artifact.send_receipt_protocol,SEND_RECEIPT_PROTOCOL);
    assert!(artifact.wasm.starts_with(b"\\0asm\\x01\\0\\0\\0"));
}
#[test]
fn await_remote_send_is_still_rejected_until_real_guest_continuations_exist() {
    let source=r#"
define actor Counter as
  receive(ActorMail<int> mail): void { return; }
end
pub fnc main(): int {
  val remote = remote_actor(Counter,1);
  val result = await remote.send_async(42);
  return 0;
}
"#;
    assert!(compile(source).is_err(),"must not silently claim async guest send support");
}
