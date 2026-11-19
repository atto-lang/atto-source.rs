//! A pinned Atto virtual NIC guest compiled by the REAL Atto browser compiler.
//! Rust is verification infrastructure, not a hardware implementation.
//!
//! The original device source blob is copied unchanged from NIC PR #3.
//! Today the browser compiler intentionally rejects int/int division (S-25),
//! so 'mac_is_group' is NOT admitted to the executable subset. Its exact
//! definition remains in the SHA-pinned source. Never claim a complete module
//! or native-KVM guest has been executed.

use std::io::Write;
use std::process::{Command, Stdio};

const NIC: &str = include_str!("fixtures/virtual_hardware/nic.atto");
const NIC_SHA: &str = "22da7b87442bd73a2fe20779e2cf875e5379397c";

fn git_blob_sha(contents: &[u8]) -> String {
    let mut p = Command::new("git")
        .args(["hash-object", "--stdin"])
        .stdin(Stdio::piped()).stdout(Stdio::piped())
        .spawn().expect("git installed");
    p.stdin.take().unwrap().write_all(contents).unwrap();
    let out = p.wait_with_output().expect("git hash-object");
    assert!(out.status.success(), "Git blob hashing failed");
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

/// Preserve the FULL original Git blob, but lower only *whole unchanged*
/// function definitions which the current browser compiler can execute.
/// Reject unlisted functions rather than accidentally "testing" a mock.
fn currently_admitted_nic_source() -> String {
    assert_eq!(git_blob_sha(NIC.as_bytes()), NIC_SHA, "NIC source drift");
    let expected = [
        "valid_mac", "mac_is_group", "valid_vlan_id", "valid_ethernet_frame",
        "valid_descriptor_ring", "can_receive", "can_transmit",
        "next_ring_slot", "available_descriptors",
    ];
    let offsets: Vec<usize> = NIC.match_indices("pub fnc ").map(|(at, _)| at).collect();
    assert_eq!(offsets.len(), expected.len(), "changed top-level API; re-audit required");
    let mut code = String::new();
    for (i, &start) in offsets.iter().enumerate() {
        let after = &NIC[start + "pub fnc ".len()..];
        let end = after.find('(').expect("typed Atto function declaration");
        let name = &after[..end];
        assert_eq!(name, expected[i], "unexpected Atto function");
        if name != "mac_is_group" {
            let limit = offsets.get(i + 1).copied().unwrap_or(NIC.len());
            code.push_str(&NIC[start..limit]); // exact unmodified function bytes
            code.push('\n');
        }
    }
    assert!(!code.contains("pub fnc mac_is_group("));
    code
}

fn compile_guest(source: &str) -> Result<Vec<u8>, String> {
    let wasm = atto_compiler::compile(source).map_err(|d| d.to_string())?;
    assert!(wasm.starts_with(b"\0asm"), "not WebAssembly");
    assert_eq!(wasm, atto_compiler::compile(source).map_err(|d| d.to_string())?,
        "non-deterministic compiled output");
    Ok(wasm)
}

fn execute_wasm_guest(wasm: &[u8]) -> i64 {
    let script = r#"
const fs = require('node:fs');
const bytes = fs.readFileSync(0);
if (!WebAssembly.validate(bytes)) throw Error('invalid Wasm');
const module = new WebAssembly.Module(bytes);
if (WebAssembly.Module.imports(module).length !== 0) throw Error('unexpected host import');
const guest = new WebAssembly.Instance(module, {});
const value = guest.exports.main();
if (typeof value !== 'bigint') throw Error('expected i64 guest ABI');
if (guest.exports.panic_code && guest.exports.panic_code.value !== 0) throw Error('guest panic');
process.stdout.write(String(value));
"#;
    let mut p = Command::new("node").args(["-e", script])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().expect("Node Wasm engine must be available");
    p.stdin.take().unwrap().write_all(wasm).unwrap();
    let out = p.wait_with_output().expect("Node guest finished");
    assert!(out.status.success(), "guest: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap().parse::<i64>().expect("guest i64")
}

#[test]
fn execute_byte_pinned_nic_functions_with_positive_and_negative_cases() {
    let source = currently_admitted_nic_source();
    let cases = [
        ("min-frame", "valid_ethernet_frame(60, 1500)"),
        ("max-frame", "valid_ethernet_frame(1514, 1500)"),
        ("short-frame-denied", "!valid_ethernet_frame(59, 1500)"),
        ("oversize-denied", "!valid_ethernet_frame(1515, 1500)"),
        ("unsupported-mtu-denied", "!valid_ethernet_frame(60, 575)"),
        ("valid-vlan", "valid_vlan_id(4094)"),
        ("invalid-vlan", "!valid_vlan_id(4095)"),
        ("invalid-mac", "!valid_mac(-1)"),
        ("max-mac", "valid_mac(281474976710655)"),
        ("rx-admit", "can_receive(100, 1500, 0, 8, true)"),
        ("rx-ring-full", "!can_receive(100, 1500, 8, 8, true)"),
        ("tx-link-down", "!can_transmit(100, 1500, 0, 8, false)"),
        ("next-ring-wrap", "next_ring_slot(7, 8) == 0"),
        ("next-ring-invalid", "next_ring_slot(8, 8) == -1"),
        ("remaining-slots", "available_descriptors(4, 8) == 4"),
        ("invalid-descriptor-count", "available_descriptors(9, 8) == -1"),
    ];
    for (name, expression) in cases {
        let guest = format!(
            "{source}\npub fnc main(): int {{ if {expression} then return 42; else return 0; fi }}\n"
        );
        let wasm = compile_guest(&guest).unwrap_or_else(|e| panic!("{name}: {e}"));
        let result = execute_wasm_guest(&wasm);
        assert_eq!(result, 42, "{name}: incorrect executed Atto guest");
        println!("ATTO_NIC_GUEST_PASS {name}");
    }
}

#[test]
fn reject_unresolved_device_capability_without_artifact() {
    let source = currently_admitted_nic_source();
    let invalid = format!(
        "{source}\npub fnc main(): int {{ return unverified_kvm_ioctl(42); }}\n"
    );
    assert!(atto_compiler::compile(&invalid).is_err(),
        "unverified KVM capability must not compile or silently generate a host stub");
}
