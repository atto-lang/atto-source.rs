// Executing mutation fuzz (local, slow; not part of CI's default run).
// Mutates every positive fixture at the token level, compiles each mutant
// with the real wasm32 compiler, validates accepted output, and EXECUTES it
// in a worker with a deadline. A trap that is not a contract panic
// (panic_code 1 or 3) or stack exhaustion is a soundness bug in the checker
// (definite-return, typing) or the emitter.
//
// usage: node tests/fuzz-exec.mjs [mutants-per-fixture=300] [seed=99]
import { readFileSync, readdirSync, mkdirSync, writeFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { classifyTimeout } from './fuzz-timeout-classifier.mjs';
import { Worker } from 'node:worker_threads';
import { join } from 'node:path';

const here = new URL('.', import.meta.url).pathname;
const perFixture = Number(process.argv[2] || 300);
let seed = Number(process.argv[3] || 99);
const initialSeed = seed;
const api = new WebAssembly.Instance(new WebAssembly.Module(readFileSync(join(here, '../target/wasm32-unknown-unknown/release/atto_compiler.wasm'))), {}).exports;
const enc = new TextEncoder();
function compile(src) {
  const b = enc.encode(src);
  if (b.length > api.input_capacity()) return null;
  new Uint8Array(api.memory.buffer, api.input_ptr(), b.length).set(b);
  if (api.compile(b.length) !== 0) return null;
  return new Uint8Array(api.memory.buffer, api.result_ptr(), api.result_length()).slice();
}
const workerSrc = `
const { parentPort } = require('node:worker_threads');
parentPort.on('message', wasm => {
  const m = new WebAssembly.Module(wasm);
  const atto = { print_i64() {}, print_bool() {}, print_str() {}, print_newline() {}, print_f64() {}, actor_spawn() { return 0; }, actor_send() {} };
  const inst = new WebAssembly.Instance(m, { atto });
  try { inst.exports.main(); parentPort.postMessage({ ok: true }); }
  catch (e) { parentPort.postMessage({ ok: false, pc: inst.exports.panic_code.value, range: e instanceof RangeError, msg: String(e) }); }
});`;
function run(wasm, budgetMs = 300) {
  return new Promise(res => {
    const w = new Worker(workerSrc, { eval: true });
    const t = setTimeout(() => { w.terminate(); res({ timeout: true }); }, budgetMs);
    w.on('message', m => { clearTimeout(t); w.terminate(); res(m); });
    w.on('error', e => { clearTimeout(t); w.terminate(); res({ ok: false, msg: 'worker ' + e }); });
    w.postMessage(wasm);
  });
}
const tokenize = s => s.match(/\s+|[A-Za-z_][A-Za-z0-9_]*|\d+(?:\.\d+)?|"(?:[^"\\\n]|\\.)*"|->|\S/g) ?? [];
const rnd = n => (seed = (seed * 48271) % 2147483647) % n;
const inserts = ['break', 'continue', 'return', 'return 0', 'loop {', '}', 'fi', 'done', 'else', 'elif true then', 'while true do',
  '-9223372036854775808', '9223372036854775807', 'match 1 _ -> { } end', '1.5', '"x"', ' + "y"', '.trim()', 'on', 'over', '_ ->', '0.0 / 0.0'];
const punct = ['(', ')', '{', '}', ';', ',', '|', '-', '0', 'true', '"', '*', '/', '->', 'end'];
let total = 0, accepted = 0, timeouts = 0, panics = 0;
const bad = [];
// Preserve exact accepted mutant sources and their RNG state when a worker
// exceeds its deadline. A timeout is *not* a soundness or liveness pass.
const timeoutRepros = [];
let expectedNonterminating = 0;
let unexpectedTimeouts = 0;
const dir = join(here, 'fixtures/pass');
for (const name of readdirSync(dir).filter(n => n.endsWith('.atto')).sort()) {
  const toks = tokenize(readFileSync(join(dir, name), 'utf8'));
  for (let k = 0; k < perFixture; k++) {
    const mutantSeed = seed;
    const t = toks.slice();
    for (let e = 1 + rnd(3); e > 0; e--) {
      const i = rnd(t.length), j = rnd(t.length);
      switch (rnd(5)) {
        case 0: t.splice(i, 1); break;
        case 1: t.splice(i, 0, t[j]); break;
        case 2: [t[i], t[j]] = [t[j], t[i]]; break;
        case 3: t[i] = inserts[rnd(inserts.length)]; break;
        default: t[i] = punct[rnd(punct.length)];
      }
    }
    total++;
    const src = t.join('');
    let w;
    try { w = compile(src); } catch (e) { bad.push(['COMPILER_TRAP', String(e), src]); break; }
    if (!w) continue;
    accepted++;
    if (!WebAssembly.validate(w)) { bad.push(['INVALID_WASM', '', src]); continue; }
    const r = await run(w);
    if (r.timeout) {
      timeouts++;
      // An additional fixed-budget replay classifies the original 300ms
      // timeout without mistaking timeout recovery for a liveness proof.
      const replay = await run(w, 3000);
      const replayResult = replay.timeout ? 'timeout_3000ms' :
        replay.ok ? 'completed_3000ms' :
        'trap_3000ms';
      const repro = {
        fixture: name, case_index: k, initial_seed: initialSeed,
        per_fixture: perFixture, rng_before_mutation: mutantSeed,
        source_sha256: createHash('sha256').update(src).digest('hex'),
        wasm_sha256: createHash('sha256').update(w).digest('hex'),
        replay_budget_ms: 3000, replay_result: replayResult,
        replay_diagnostic: !replay.timeout && !replay.ok ? String(replay.msg) : null,
        source: src,
      };
      repro.classification = classifyTimeout(repro);
      timeoutRepros.push(repro);
      if (repro.classification.status === 'expected_nonterminating_mutant') {
        expectedNonterminating++;
      } else {
        unexpectedTimeouts++;
        bad.push(['UNCLASSIFIED_TIMEOUT', replayResult, src]);
      }
    } else if (!r.ok) {
      if (r.pc === 1 || r.pc === 3 || r.range) panics++;
      else bad.push(['UNSOUND_TRAP', r.msg, src]);
    }
  }
}
if (timeouts) {
  const outDir = join(here, '../dist');
  mkdirSync(outDir, { recursive: true });
  writeFileSync(join(outDir, 'mutation-timeout-repros.json'),
    JSON.stringify({ initial_seed: initialSeed, per_fixture: perFixture,
      timeout_ms: 300, cases: timeoutRepros }, null, 2) + '\n');
  console.log('TIMEOUT_REPROS dist/mutation-timeout-repros.json');
  for (const c of timeoutRepros.slice(0, 5)) {
    console.log('TIMEOUT_CASE', c.fixture, c.case_index, c.source_sha256,
      'REPLAY', c.replay_result, 'CLASSIFIED', c.classification.status);
  }
}
console.log(JSON.stringify({
  total, accepted, timeouts,
  expected_nonterminating_mutants: expectedNonterminating,
  unclassified_timeouts: unexpectedTimeouts, panics, bad: bad.length,
}));
for (const b of bad.slice(0, 5)) console.log(b[0], b[1], '\n' + b[2] + '\n-----');
process.exit(bad.length ? 1 : 0);
