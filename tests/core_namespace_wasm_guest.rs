//! Real Wasm guest execution for namespace-only scalar Atto core functions.
//! Use explicit opt-in so ordinary Rust-only builds need no Node.js installation.
use atto_compiler::linker::{compile_linked, LinkUnit};
use std::io::Write;
use std::process::{Command, Stdio};

const SLICE: &str = include_str!("../core/slice.atto");
const ITER: &str = include_str!("../core/iter.atto");
const CMP: &str = include_str!("../core/cmp.atto");

const INTEGRATED: &str = r#"use namespace app.entry;
import namespace slice from "+core/slice.atto";
import namespace iter from "+core/iter.atto";
import namespace cmp from "+core/cmp.atto";
pub fnc main(): int {
  if !slice.valid_range(0, 2, 2) then return 1; fi
  if slice.valid_range(1, 2, 2) then return 2; fi
  match iter.next_index(1, 2, 0, 3) over
    on Some(n) -> { if n != 1 then return 3; fi }
    on None -> { return 4; }
  end
  if cmp.compare_int(9223372036854775807, -9223372036854775807) != 1 then return 5; fi
  return 42;
}"#;

const EXHAUSTED: &str = r#"use namespace app.entry;
import namespace slice from "+core/slice.atto";
import namespace iter from "+core/iter.atto";
import namespace cmp from "+core/cmp.atto";
pub fnc main(): int {
  match iter.next_index(1, 2, 2, 3) over
    on Some(n) -> { return 1; }
    on None -> { return 42; }
  end
}"#;

const JS: &str = r#"
const parts=[];
process.stdin.on('data', chunk=>parts.push(chunk));
process.stdin.on('end', async ()=>{
  try {
    const bytes=Buffer.concat(parts);
    const module=await WebAssembly.compile(bytes);
    if(WebAssembly.Module.imports(module).length!==0)
      throw new Error('unexpected host imports in freestanding scalar guest');
    const instance=await WebAssembly.instantiate(module,{});
    if(typeof instance.exports.main!=='function')
      throw new Error('guest main is not exported');
    const output=instance.exports.main();
    if(output!==42n)
      throw new Error('guest main expected BigInt 42, got '+String(output));
    if(instance.exports.panic_code?.value!==0)
      throw new Error('guest panic code nonzero');
    process.stdout.write('ATTO_WASM_MAIN_EXECUTED_42\n');
  } catch(err) {
    console.error(err);
    process.exitCode=1;
  }
});
"#;

fn guest(entry: &str) {
    let units = [
        LinkUnit { path: "entry.atto", package: "app@1.0.0", source: entry },
        LinkUnit { path: "core/slice.atto", package: "app@1.0.0", source: SLICE },
        LinkUnit { path: "core/iter.atto", package: "app@1.0.0", source: ITER },
        LinkUnit { path: "core/cmp.atto", package: "app@1.0.0", source: CMP },
    ];
    let wasm = compile_linked("entry.atto", &units).expect("verified namespace source graph");
    assert!(wasm.starts_with(b"\0asm\x01\0\0\0"));
    let mut child = Command::new("node")
        .args(["-e", JS])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Node.js WebAssembly runner is required for this explicit test");
    child.stdin.take().unwrap().write_all(&wasm).expect("send exact compiled guest bytes");
    let outcome = child.wait_with_output().expect("Node Wasm guest completion");
    assert!(outcome.status.success(), "Node Wasm guest failed: {}",
        String::from_utf8_lossy(&outcome.stderr));
    assert_eq!(outcome.stdout, b"ATTO_WASM_MAIN_EXECUTED_42\n");
}

#[test]
#[ignore = "requires Node.js runtime; exact-head Actions invokes this explicitly"]
fn real_wasm_guest_executes_bounds_cursor_and_ordering() {
    guest(INTEGRATED);
}

#[test]
#[ignore = "requires Node.js runtime; exact-head Actions invokes this explicitly"]
fn real_wasm_guest_exhausted_cursor_uses_none_branch() {
    guest(EXHAUSTED);
}
