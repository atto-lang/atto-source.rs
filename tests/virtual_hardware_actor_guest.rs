//! Exercises real Atto actor receive handlers in separate WebAssembly instances.
//! The Node host is intentionally serial and deny-by-default. This proves
//! guest message ordering/state and stale-ticket rejection, NOT threading, KVM,
//! production authentication, native AOT or a full actor supervisor runtime.
use std::io::Write;
use std::process::{Command, Stdio};

const NIC: &str = include_str!("virtual-hardware/nic_actor.atto");
const ROUTER: &str = include_str!("virtual-hardware/router_actor.atto");
const SWITCH: &str = include_str!("virtual-hardware/switch_actor.atto");
const MODEM: &str = include_str!("virtual-hardware/modem_actor.atto");

fn run_actor(name: &str, actor_type: &str, actor_src: &str,
             mail: &[i64], expected_lines: &[&str]) {
    let sends = mail.iter().map(|value| format!("  device.send({value});"))
        .collect::<Vec<_>>().join("\n");
    let guest = format!("{actor_src}\npub fnc main(): int {{\n  val device = spawn {actor_type}();\n{sends}\n  return 0;\n}}\n");
    let wasm = atto_compiler::compile(&guest)
        .unwrap_or_else(|err| panic!("{name}: failed to compile real actor source: {err}"));
    assert!(wasm.starts_with(b"\0asm\x01\0\0\0"), "actor Wasm magic");
    assert_eq!(wasm, atto_compiler::compile(&guest).expect("deterministic actor compilation"));
    let script = r#"
const fs = require('node:fs');
const bytes = fs.readFileSync(0);
if (!WebAssembly.validate(bytes)) throw Error('invalid actor guest wasm');
const module = new WebAssembly.Module(bytes);
const imports = new Set(['print_i64','print_bool','print_str','print_newline',
                         'print_f64','actor_spawn','actor_send']);
for (const imp of WebAssembly.Module.imports(module)) {
  if (imp.module !== 'atto' || !imports.has(imp.name))
    throw Error('untrusted import ' + imp.module + '.' + imp.name);
}
const actors = [], lines = [];
let line = '';
const decoder = new TextDecoder();
function host(holder) {
  return {
    print_i64: val => { line += String(val); },
    print_bool: val => { line += val ? 'true' : 'false'; },
    print_f64: val => { line += String(val); },
    print_str: (off,len) => {
      const memory = holder.inst?.exports?.memory;
      if (!memory) throw Error('missing per-instance guest memory');
      line += decoder.decode(new Uint8Array(memory.buffer,off,len));
    },
    print_newline: () => { lines.push(line); line=''; },
    actor_spawn: type => {
      if (actors.length >= 8) throw Error('exhausted bounded actor host slots');
      const h = {inst:null};
      const inst = new WebAssembly.Instance(module,{atto:host(h)});
      h.inst=inst;
      if (!inst.exports.actor_init || !inst.exports.actor_receive)
        throw Error('missing actual actor ABI exports');
      inst.exports.actor_init(type);
      const id = actors.length;
      actors.push({type,inst,queue:[]});
      return id;
    },
    actor_send: (ref,value) => {
      const actor=actors[ref];
      if (!actor || actor.queue.length >= 128) throw Error('invalid or full mailbox');
      actor.queue.push(value);
    },
  };
}
const mainHost = {inst:null};
const main = new WebAssembly.Instance(module,{atto:host(mainHost)});
mainHost.inst=main;
if (String(main.exports.main()) !== '0') throw Error('Atto main failed');
for (const actor of actors) {
  if (actor.inst === main) throw Error('actor not isolated');
  let sequence=0n;
  for (const value of actor.queue)
    actor.inst.exports.actor_receive(actor.type,value,sequence++);
  if (actor.inst.exports.panic_code.value !== 0)
    throw Error('actor panic code ' + actor.inst.exports.panic_code.value);
}
if (main.exports.panic_code.value !== 0) throw Error('main panic');
if (line !== '') throw Error('unterminated output');
const got = lines.join(',');
if (got !== process.argv[1])
  throw Error('actor output mismatch: got ['+got+'] expected ['+process.argv[1]+']');
console.log('VHW_ACTOR_EXECUTED '+lines.length+' lines panic=0 actors='+actors.length);
"#;
    let expected = expected_lines.join(",");
    let mut child = Command::new("node").args(["-e", script, &expected])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().expect("Node.js is required for real actor WebAssembly tests");
    child.stdin.take().unwrap().write_all(&wasm).unwrap();
    let result = child.wait_with_output().expect("actor execution result");
    assert!(result.status.success(), "{name}: actual actor guest failed: {}",
            String::from_utf8_lossy(&result.stderr));
    let out = String::from_utf8(result.stdout).expect("UTF8 actor receipt");
    assert!(out.contains("VHW_ACTOR_EXECUTED"), "{name}: missing host receipt: {out}");
    println!("VHW_ACTOR_GUEST name={name} {}", out.trim());
}

#[test]
fn nic_actor_link_generation_rejects_stale_completion() {
    run_actor("nic", "VirtualNic", NIC,
        &[60, -5, 65537, 60, -2, -3, 60, 65537, -4, 131073, -4],
        &["1","1","0","1","-2","-3","1","-1","1","1","0","0","1"]);
}
#[test]
fn router_actor_routes_and_rejects_special_destinations() {
    run_actor("router", "VirtualRouter", ROUTER,
        &[167837697, 167772161, 2130706433, -1, -2],
        &["3","2","-1","-1","2","2","-1"]);
}
#[test]
fn switch_actor_traps_control_and_expires_learned_fdb() {
    run_actor("switch", "VirtualSwitch", SWITCH,
        &[-1, 2199023255553, 1652522221568, -2, 2199023255553,
          -3, 2199023255553, -4],
        &["2","2","-1","-2","-2","-3","-2","1"]);
}
#[test]
fn modem_actor_rejects_old_epoch_ack_and_legacy_commands() {
    run_actor("modem", "VirtualModem", MODEM,
        &[1,2,3,4,8,24,40,9,25,41,40,5,41,-2,-3],
        &["1","2","3","4","-1","-1","64","-1","-1","0","64","1","-1","3","5"]);
}
