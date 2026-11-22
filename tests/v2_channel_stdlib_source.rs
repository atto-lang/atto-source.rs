//! Source-package guard, not a substitute for guest execution tests.
const LIB: &str = include_str!("../stdlib/channel.atto");
const PRELUDE: &str = include_str!("../src/v2/prelude.atto");
const SLOT: &str = "// @atto-stdlib-channel-source";

#[test]
fn channel_module_is_extracted_once_and_not_duplicated_in_prelude() {
    assert_eq!(PRELUDE.matches(SLOT).count(), 1);
    assert!(!PRELUDE.contains("fnc __chan_new("));
    assert!(!PRELUDE.contains("fnc __chan_send("));
    let declarations = [
        "__chan_new", "__chan_offer", "__chan_send", "__chan_send_pipe",
        "__chan_take", "__chan_recv", "__chan_recv_user_opt",
        "__chan_recv_opt", "__chan_try", "__chan_nb",
        "__chan_close_internal", "__chan_close",
    ];
    for name in declarations {
        assert_eq!(LIB.matches(&format!("fnc {name}(")).count(), 1,
            "the channel source must define {name} exactly once");
    }
    assert!(!LIB.contains("import java:"));
    assert!(!LIB.contains("import jvm"));
    assert!(!LIB.contains("require("));
    assert!(!LIB.contains("host_call"));
}

#[test]
fn channel_module_is_compiler_injected_at_the_original_runtime_boundary() {
    let composed = PRELUDE.replacen(SLOT, LIB, 1);
    assert!(composed.contains("fnc __chan_close(int c): void"));
    assert!(composed.contains("fnc __actor_stop(int a): void"));
    assert!(!composed.contains(SLOT));
    let channel = composed.find("fnc __chan_new(int cap)").unwrap();
    let actors = composed.find("// ------------------------------------------------------------------ actors").unwrap();
    let futures = composed.find("// ----------------------------------------------------------------- futures").unwrap();
    assert!(futures < channel && channel < actors);
}
