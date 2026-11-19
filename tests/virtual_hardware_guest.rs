//! Exact-source Atto virtualization fixtures compiled to WASM and actually executed.
//! Rust is the test runner only; device logic is 1:1 source from four PRs.
//! Captured Git blob identities are checked by the companion workflow.
use std::io::Write;
use std::process::{Command, Stdio};

const NIC: &str = include_str!("virtual-hardware/nic.atto");
const ROUTER: &str = include_str!("virtual-hardware/router.atto");
const SWITCH: &str = include_str!("virtual-hardware/switch.atto");
const MODEM: &str = include_str!("virtual-hardware/modem.atto");

// The Wasm guest must run every positive and adversarial predicate itself.
// Do not substitute a Rust computation for any Atto function result.
const NIC_MAIN: &str = r#"
if guest_region_contains(64, 60, 124) and
   !guest_region_contains(64, 60, 123) and
   guest_address_aligned(64, 64) and
   !guest_address_aligned(65, 64) and
   can_receive_guest_frame(64, 60, 124, 64, 1500, 0, 8, true) and
   !can_receive_guest_frame(64, 60, 123, 64, 1500, 0, 8, true) then
  return 41;
fi
return -1;
"#;
const ROUTER_MAIN: &str = r#"
if canonical_ipv4_network(3232235776, 24) and
   !canonical_ipv4_network(3232235777, 24) and
   !canonical_ipv4_network(1, 0) and
   may_forward_on_link(3232235777, 1500, 20, 2, 0, 1500, true) and
   !may_forward_on_link(3232235777, 1501, 20, 2, 0, 1500, true) and
   !may_forward_on_link(3758096385, 20, 20, 64, 0, 1500, true) and
   select_canonical_egress_port(3232235777, 0, 0, 1, 3232235776, 24, 3) == 3 then
  return 42;
fi
return -1;
"#;
const SWITCH_MAIN: &str = r#"
if can_admit_ingress_frame(60, 1500, 0, 2, 1, false, true) and
   !can_admit_ingress_frame(60, 1500, 0, 2, 1, true, true) and
   !can_admit_ingress_frame(60, 1500, 0, 2, 4095, false, true) and
   valid_unicast_destination_mac(2199023255553) and
   !valid_unicast_destination_mac(0) and
   select_safe_egress_port(0, 0, 2, 1, 2199023255553, 1, 1, 0, 300) == -1 and
   select_safe_egress_port(2199023255553, 0, 2, 1, 2199023255553, 1, 1, 0, 300) == 1 then
  return 43;
fi
return -1;
"#;
const MODEM_MAIN: &str = r#"
if valid_serial_config(115200, 8, 0, 1) and
   !valid_serial_config(115201, 8, 0, 1) and
   free_modem_fifo(0, 8) == 8 and
   free_modem_fifo(8, 8) == 0 and
   next_modem_fifo_slot(7, 8) == 0 and
   can_transmit_octet(4, 255, 0, 8, true, true) and
   !can_transmit_octet(4, 256, 0, 8, true, true) and
   can_admit_at_command(1, 2, 0, 8, true, true) and
   !can_admit_at_command(4, 2, 0, 8, true, true) then
  return 44;
fi
return -1;
"#;

#[test]
fn compile_and_execute_pr_sources_on_tip() {
    let fixtures = [
        ("nic", NIC, NIC_MAIN, 41_i64),
        ("router", ROUTER, ROUTER_MAIN, 42_i64),
        ("switch", SWITCH, SWITCH_MAIN, 43_i64),
        ("modem", MODEM, MODEM_MAIN, 44_i64),
    ];
    for (name, source, body, expected) in fixtures {
        let guest = format!("{source}\npub fnc main(): int {{\n{body}\n}}\n");
        let wasm = atto_compiler::compile(&guest)
            .unwrap_or_else(|d| panic!("{name}: Atto compiler rejected exact PR source: {d}"));
        assert!(wasm.starts_with(b"\0asm\x01\0\0\0"), "{name}: invalid Wasm header");
        assert_eq!(wasm, atto_compiler::compile(&guest).expect("determinism"),
                   "{name}: nondeterministic Wasm");
        let runner = r#"
const fs = require('node:fs');
const wasm = fs.readFileSync(0);
if (!WebAssembly.validate(wasm)) throw Error('invalid guest Wasm');
const module = new WebAssembly.Module(wasm);
const allowed = new Set(['print_i64','print_bool','print_str','print_newline','print_f64',
  'actor_spawn','actor_send','state_register','state_enter','state_leave']);
for (const imp of WebAssembly.Module.imports(module))
  if (imp.module !== 'atto' || !allowed.has(imp.name))
    throw Error('unrecognized host import: '+imp.module+'.'+imp.name);
const deny = () => { throw Error('unexpected host effect'); };
const host = Object.fromEntries([...allowed].map(name => [name, deny]));
const inst = new WebAssembly.Instance(module, {atto:host});
const got = inst.exports.main();
if (String(got) !== process.argv[1]) throw Error('guest returned '+got+' expected '+process.argv[1]);
if (inst.exports.panic_code.value !== 0) throw Error('guest panicked');
console.log('REAL_ATTO_GUEST_EXECUTED '+String(got)+' panic=0');
"#;
        let mut child = Command::new("node").args(["-e", runner, &expected.to_string()])
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
            .spawn().expect("Node.js required for guest Wasm execution");
        child.stdin.take().expect("Wasm input")
            .write_all(&wasm).expect("send guest Wasm to Node");
        let output = child.wait_with_output().expect("collect guest execution");
        assert!(output.status.success(), "{name}: real compiled guest failed: {}",
                String::from_utf8_lossy(&output.stderr));
        let out = String::from_utf8(output.stdout).expect("UTF-8 guest evidence");
        assert!(out.contains(&format!("REAL_ATTO_GUEST_EXECUTED {expected} panic=0")),
                "{name}: missing real execution evidence: {out}");
        println!("VHW_GUEST name={name} result={expected} panic=0 bytes={} kind=SOURCE_COMPILED_WASM_GUEST",
                 wasm.len());
    }
}
