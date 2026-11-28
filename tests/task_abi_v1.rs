//! atto.task.v1: compiled `susp` callables executed by an independent,
//! minimal reference scheduler (Node, virtual clock). The production
//! browser scheduler is OresScheduler in atto-lang.github.io/play; this
//! driver only implements the documented step contract (docs/TASK_ABI_V1.md)
//! so the compiler's guest output is checked without that repository.
use atto_compiler::compile;
use std::io::Write;
use std::process::{Command, Stdio};

const DRIVER: &str = r#"
const fs = require('node:fs');
const wasm = fs.readFileSync(0);
if (!WebAssembly.validate(wasm)) throw Error('invalid Wasm');
const mod = new WebAssembly.Module(wasm);
const marker = WebAssembly.Module.customSections(mod, 'atto.task.v1');
if (marker.length !== 1 || new Uint8Array(marker[0])[0] !== 1) throw Error('missing atto.task.v1 marker');
const names = WebAssembly.Module.exports(mod).map(e => e.name);
if (names.includes('main')) throw Error('task guest must not export main');
let out = '';
let memory;
const atto = {
  print_i64: v => { out += String(v); }, print_bool: v => { out += v ? 'true' : 'false'; },
  print_newline: () => { out += '\n'; }, print_f64: v => { out += String(v); },
  print_str: (p, n) => { out += new TextDecoder().decode(new Uint8Array(memory.buffer, p, n)); },
};
const imports = {};
for (const imp of WebAssembly.Module.imports(mod)) {
  if (imp.module !== 'atto' || !(imp.name in atto)) throw Error('unexpected import ' + imp.name);
  (imports.atto ??= {})[imp.name] = atto[imp.name];
}
const inst = new WebAssembly.Instance(mod, imports);
memory = inst.exports.memory;
const x = inst.exports;
const tasks = new Map(); const futures = new Set(); const ready = []; const timers = [];
let next = 1, now = 0, turns = 0, live = 0;
const admit = (h, kind) => { const id = next++; tasks.set(id, { id, h, kind, wake: 0n, waiters: [], done: false }); live++; return id; };
const wakeUp = (t, v, front) => { t.wake = v; front ? ready.unshift(t.id) : ready.push(t.id); };
const root = admit(x.atto_task_main(), 'root'); ready.push(root);
for (;;) {
  const id = ready.shift();
  if (id === undefined) {
    if (!timers.length) { if (live) throw Error('deadlock'); break; }
    timers.sort((a, b) => a.at - b.at || a.seq - b.seq);
    const t = timers.shift(); now = Math.max(now, t.at); wakeUp(tasks.get(t.id), 0n, false); continue;
  }
  const t = tasks.get(id); const w = t.wake; t.wake = 0n;
  if (++turns > 1e6) throw Error('turn budget');
  const s = x.atto_task_resume(t.h, w);
  if (s === 0) {
    t.done = true; t.result = x.atto_task_result(t.h); x.atto_task_close(t.h); live--;
    for (const wid of t.waiters) wakeUp(tasks.get(wid), t.result, t.kind === 'call');
  } else if (s === 1) {
    const c = admit(x.atto_task_request(t.h), 'call'); tasks.get(c).waiters.push(id); ready.unshift(c);
  } else if (s === 2) {
    const f = Number(x.atto_task_request(t.h)); if (!futures.has(f)) throw Error('bad future');
    const target = tasks.get(f); if (target.done) wakeUp(t, target.result, false); else target.waiters.push(id);
  } else if (s === 3 || s === 4) {
    const c = admit(x.atto_task_request(t.h), s === 3 ? 'task' : 'detached');
    if (s === 3) futures.add(c); ready.push(c); wakeUp(t, s === 3 ? BigInt(c) : 0n, true);
  } else if (s === 5) {
    timers.push({ id, at: now + Number(x.atto_task_request(t.h)), seq: turns });
  } else if (s === 6) { wakeUp(t, 0n, false); }
  else throw Error('bad status ' + s);
}
if (x.panic_code.value !== 0) throw Error('panicked');
process.stdout.write(JSON.stringify({ value: String(tasks.get(root).result), stdout: out, now }));
"#;

fn run(src: &str) -> String {
    let wasm = compile(src).unwrap_or_else(|d| panic!("compile rejected task guest: {d}"));
    assert_eq!(wasm, compile(src).unwrap(), "deterministic output");
    let mut child = Command::new("node").args(["-e", DRIVER])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().expect("Node is required for real Wasm guest execution");
    child.stdin.take().unwrap().write_all(&wasm).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "task guest failed: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn task_await_and_detached_task_run_on_scheduler() {
    let got = run(include_str!("fixtures/task/tasks-async.atto"));
    assert_eq!(got, r#"{"value":"42","stdout":"42\n","now":0}"#);
}

#[test]
fn sleeper_suspends_on_a_host_timer() {
    let got = run(include_str!("fixtures/task/timers-sleep.atto"));
    assert_eq!(got, r#"{"value":"42","stdout":"","now":100}"#);
}

#[test]
fn lexical_borrow_ends_before_await() {
    let got = run(include_str!("fixtures/task/borrow-suspension.atto"));
    assert_eq!(got, r#"{"value":"43","stdout":"","now":0}"#);
}

#[test]
fn loops_recursion_strings_and_interleaving_survive_suspension() {
    let got = run(include_str!("fixtures/task/interleaving.atto"));
    // worker(10, 20ms) wakes at 20/40/60 and worker(1, 30ms) at 30/60/90.
    // At t=60 both are due; the timer registered first (at t=30) runs first.
    assert_eq!(got, r#"{"value":"156","stdout":"120\ntrue\n10\n1\n10\n1\n10\n1\nsum\n33\n10000\ndetached\n","now":90}"#);
}

#[test]
fn ordinary_modules_are_unchanged_by_the_task_abi() {
    let wasm = compile("pub fnc main(): int { return 42; }").unwrap();
    let marker = b"atto.task.v1";
    assert!(!wasm.windows(marker.len()).any(|w| w == marker));
}

#[test]
fn susp_main_may_spawn_and_mail_actors() {
    // Actor delivery is the host Mailman's job (OresVM lane); here we check
    // the guest carries both the actor and the task ABI surfaces.
    let wasm = compile(include_str!("fixtures/task/actors-interleave.atto")).expect("compiles");
    for needle in [&b"atto.task.v1"[..], b"atto.actors", b"actor_receive", b"atto_task_resume", b"actor_send"] {
        assert!(wasm.windows(needle.len()).any(|w| w == needle), "missing {}", String::from_utf8_lossy(needle));
    }
}

#[test]
fn direct_call_suspending_lambdas_lower_to_susp_callables() {
    // typed `suspends |int n| -> n + 1` with a captured task Future upstream
    let got = run(include_str!("fixtures/task/suspends.atto"));
    assert_eq!(got, r#"{"value":"42","stdout":"42\n","now":0}"#);
    // untyped `susp |x| : int -> x + 1`, parameter type fixed by its call
    let got = run(include_str!("fixtures/task/susp-shorthand.atto"));
    assert_eq!(got, r#"{"value":"43","stdout":"","now":0}"#);
}
