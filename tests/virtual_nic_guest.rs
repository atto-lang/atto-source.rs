//! Compile byte-identical Atto virtual NIC source from the device PR and run
//! actual compiler-produced WebAssembly. Rust is test orchestration only.
//! The fixture SHA pins the source Git blob, not just a paraphrased oracle.
//! No Python; Node's Wasm engine is the existing browser backend host.
use std::io::Write;
use std::process::{Command, Stdio};

const NIC: &str = include_str!("fixtures/virtual_hardware/nic.atto");
const NIC_SHA: &str = "22da7b87442bd73a2fe20779e2cf875e5379397c";

fn git_blob_sha(contents: &[u8]) -> String {
    let mut p = Command::new("git").args(["hash-object", "--stdin"])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().expect("git installed");
    p.stdin.take().unwrap().write_all(contents).unwrap();
    let out = p.wait_with_output().expect("git hash-object");
    assert!(out.status.success());
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

fn run_guest(source: &str) -> Result<i64, String> {
    let wasm = atto_compiler::compile(source).map_err(|d| d.to_string())?;
    assert!(wasm.starts_with(b"\0asm"), "real wasm magic");
    let script = r#"
const fs=require('node:fs');
const bytes=fs.readFileSync(0);
if (!WebAssembly.validate(bytes)) throw Error('invalid guest module');
const m=new WebAssembly.Module(bytes);
if (WebAssembly.Module.imports(m).length !== 0) throw Error('no host imports permitted');
const g=new WebAssembly.Instance(m,{});
const value=g.exports.main();
if (g.exports.panic_code && g.exports.panic_code.value !== 0) throw Error('guest panic');
process.stdout.write(String(value));
"#;
    let mut p=Command::new("node").args(["-e",script])
      .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
      .spawn().expect("Node.js Wasm runtime available in compiler CI");
    p.stdin.take().unwrap().write_all(&wasm).unwrap();
    let out=p.wait_with_output().unwrap();
    assert!(out.status.success(), "guest execution: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap().parse::<i64>().map_err(|e| e.to_string())
}

#[test]
fn compile_exact_virtual_nic_and_run_admission_checks() {
    assert_eq!(git_blob_sha(NIC.as_bytes()), NIC_SHA, "NIC source bytes changed");
    let guest = format!(r#"{NIC}
pub fnc main(): int {{
  if !valid_ethernet_frame(60, 1500) then return 1; fi
  if !valid_ethernet_frame(1514, 1500) then return 2; fi
  if valid_ethernet_frame(1515, 1500) then return 3; fi
  if valid_ethernet_frame(59, 1500) then return 4; fi
  if !valid_vlan_id(4094) then return 5; fi
  if valid_vlan_id(4095) then return 6; fi
  if !can_receive(100, 1500, 0, 8, true) then return 7; fi
  if can_receive(100, 1500, 8, 8, true) then return 8; fi
  if can_transmit(100, 1500, 0, 8, false) then return 9; fi
  if next_ring_slot(7, 8) != 0 then return 10; fi
  if available_descriptors(4, 8) != 4 then return 11; fi
  return 42;
}}
"#);
    assert_eq!(run_guest(&guest).unwrap(), 42);
}

#[test]
fn reject_invalid_atto_guest_without_wasm_artifact() {
    let invalid = format!("{NIC}\npub fnc main(): int {{ return undeclared_descriptor; }}\n");
    assert!(atto_compiler::compile(&invalid).is_err(),
            "compiler must reject invalid guest rather than emit Wasm");
}
