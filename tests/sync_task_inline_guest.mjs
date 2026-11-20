// Execute true native-compiled sync task guests, not parser-only checks.
import assert from 'node:assert/strict';
import { mkdtemp, writeFile, readFile, rm, stat } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const attoc = process.env.ATTOC || fileURLToPath(new URL('../target/debug/attoc', import.meta.url));
const pass = [
  ['do sync task {}; return 42;', 42n],
  ['do sync task<void> {}; return 42;', 42n],
  ['do sync task { return; }; return 42;', 42n],
  ['do sync task { defer {}; return; }; return 42;', 42n],
  [`do sync task { ${'defer {};'.repeat(32)} return; }; return 42;`, 42n],
  [`let x = 0; do sync task { defer { ${'x = 0;'.repeat(64)} }; return; }; return x + 42;`, 42n],
  ['let x = 0; do sync task { defer { x = 42; }; return; }; return x;', 42n],
  ['let x = 0; do sync task<void> { defer { x = 1; }; defer { x = 2; }; return; }; return 41 + x;', 42n],
  ['let x = 0; do sync task { defer { x = 42; }; }; return x;', 42n],
  ['let x = 0; val v = sync task<int> { defer { x = 2; }; return 40; }; return v + x;', 42n],
  ['let x = 0; val v = sync task<int> { defer { x = 2; }; if true { return 40; } else { return 1; } fi }; return v + x;', 42n],
  ['do sync task<void> { return; }; return 42;', 42n],
  ['do /* gap */ sync /* gap */ task<void> { return; }; return 42;', 42n],
  ['block { do sync task {}; } return 42;', 42n],
  ['val x = sync task<int> { return 40 + 2; }; return x;', 42n],
  ['val x = sync task<int> { val a = 40; let b = a + 1; b = b + 1; return b; }; return x;', 42n],
  ['val x = sync task<int> { val a = 40; if a == 40 { return a + 2; } else { return 7; } fi }; return x;', 42n],
  ['val x = sync task<int> { block { val y = 42; return y; } }; return x;', 42n],
  ['let x = 40; do sync task { x = x + 2; return; }; return x;', 42n],
  ['val x = sync task<int> { if true { return 40; } else { return 1; } fi }; return x + 2;', 42n],
  ['val x = sync task<int> { if false { return 1; } else { return 40; } fi }; return x + 2;', 42n],
  ['let x = 40; do sync task { return; x = 0; }; return x + 2;', 42n],
  ['do sync task { val x = 4; if x == 4 { return; } fi }; return 42;', 42n],
  ['val x = sync task<int> { if true { return 42; } else { return 7; } fi }; return x;', 42n],
  ['val x = 40; return sync task<int> { return x + 2; };', 42n],
  ['do sync task {}; val result = sync task<int> { return 40 + 2; }; do sync task {}; return result;', 42n],
  ['val x = sync task<bool> { return true; }; return 42;', 42n],
  ['val s = sync task<string> { return "safe"; }; return 42;', 42n],
  ['val x = sync task<int> { if "ok" == "ok" { return 42; } else { return 0; } fi }; return x;', 42n],
];
const fail = [
  ['sync task {}; return 42;', 'E_SYNTAX'],
  ['sync task<void> {}; return 42;', 'E_SYNTAX'],
  ['sync task<int> { return 42; }; return 42;', 'E_SYNTAX'],
  ['(sync task {}); return 42;', 'E_SYNTAX'],
  ['((sync task<void> {})); return 42;', 'E_SYNTAX'],
  ['(sync task<int> { return 42; }); return 42;', 'E_SYNTAX'],
  ['do sync task<int> {}; return 42;', 'E_TASK_RETURN_TYPE'],
  ['do sync task<bool> {}; return 42;', 'E_TASK_RETURN_TYPE'],
  ['do sync task<int> { return 7; }; return 42;', 'E_TASK_RETURN_TYPE'],
  ['do sync task { return 7; }; return 42;', 'E_TASK_RETURN_TYPE'],
  ['do sync task<void> { return 7; }; return 42;', 'E_TASK_RETURN_TYPE'],
  ['do sync task<bool> { return; }; return 42;', 'E_TASK_RETURN_TYPE'],
  ['do sync task<int> { defer { } return 42; }; return 42;', 'E_TASK_RETURN_TYPE'],
  ['do sync task { return sync task<int> { return 42; }; }; return 42;', 'E_TASK_RETURN_TYPE'],
  ['val x = sync task<void> {}; return 42;', 'E_TYPE'],
  ['let x = sync task {}; return 42;', 'E_TYPE'],
  ['val x = sync task<int> { return true; }; return 42;', 'E_TYPE'],
  ['val x = sync task<int> { val y = 42; }; return x;', 'E_RETURN'],
  ['val x = sync task<int> { if true { return 42; } fi }; return x;', 'E_RETURN'],
  ['val x = sync task<int> { if true { return 42; } else { return false; } fi }; return x;', 'E_TYPE'],
  ['do sync task<void> { if true { return 42; } fi }; return 0;', 'E_TASK_RETURN_TYPE'],
  ['val x = sync task<int> { val a = 1; block { val b = 2; } return b; }; return x;', 'E_UNBOUND'],
  ['do sync task { if true { return 7; } else { return; } fi }; return 0;', 'E_TASK_RETURN_TYPE'],
  ['val x = sync task<int> { return 42; return false; }; return x;', 'E_TYPE'],
  ['val x = sync task<int> { while true { return 42; } }; return x;', 'E_UNSUPPORTED'],
  ['do sync task { val x = stdio.println(42); }; return 42;', 'E_UNSUPPORTED'],
  ['val x = sync task<int> { return spawn Worker(); }; return x;', 'E_UNSUPPORTED'],
  ['val x = sync task<int> { return stdio.println(42); }; return x;', 'E_UNSUPPORTED'],
  ['val x = sync task<string> { return "a" + "b"; }; return 42;', 'E_UNSUPPORTED'],
  ['val x = sync task<string> { val a = "a"; return a + "b"; }; return 42;', 'E_UNSUPPORTED'],
  ['val x = sync task<int> { if ("a" + "b") == "ab" { return 42; } else { return 0; } fi }; return x;', 'E_UNSUPPORTED'],
  ['val x = sync task<int> { return await task<int> { return 42; }; }; return x;', 'E_UNSUPPORTED'],
  ['val x = task<int> { return 42; }; return 0;', 'E_UNSUPPORTED'],
  ['let x = 0; do sync task { defer { x = x + 1; }; }; return x;', 'E_UNSUPPORTED'],
  [`do sync task { ${'defer {};'.repeat(33)} return; }; return 42;`, 'E_SOURCE_LIMIT'],
  [`let x = 0; do sync task { defer { ${'x = 0;'.repeat(65)} }; }; return x;`, 'E_SOURCE_LIMIT'],
  ['val x = 0; do sync task { defer { x = 42; }; }; return x;', 'E_IMMUTABLE'],
  ['do sync task { defer { x = 42; }; let x = 0; }; return 0;', 'E_UNBOUND'],
  ['let x = 0; do sync task { defer (|int v| -> _ -> { x = v; })(42); }; return x;', 'E_UNSUPPORTED'],
  ['let x = 0; do sync task { defer { x = stdio.println(42); }; }; return x;', 'E_UNSUPPORTED'],
  ['let x = 0; do sync task { defer { x = 1; }; x = x + 1; }; return x;', 'E_UNSUPPORTED'],
  ['let x = 0; do sync task { x = 1; defer { x = 2; }; }; return x;', 'E_UNSUPPORTED'],
  ['let x = 0; do sync task { block { defer { x = 2; }; } }; return x;', 'E_UNSUPPORTED'],
  ['let x = 0; do sync task { defer { val y = 42; }; }; return x;', 'E_UNSUPPORTED'],
  ['let x = 0; do sync task { defer { x = borrow! x; }; }; return x;', 'E_UNSUPPORTED'],
  ['let x = 0; do sync task { defer { x = 2; }; if x == 0 { return; } fi }; return x;', 'E_UNSUPPORTED'],
  ['let x = 0; do sync task { defer { x = 2; }; return sync task<int> { return 42; }; }; return x;', 'E_TASK_RETURN_TYPE'],
];
const work = await mkdtemp(join(tmpdir(), 'atto-sync-task-'));
try {
  let i = 0;
  for (const [body, expected] of pass) {
    const input = join(work, `pass-${i}.atto`);
    const out = join(work, `pass-${i++}.wasm`);
    await writeFile(input, `pub fnc main(): int { ${body} }\n`);
    const res = spawnSync(attoc, [input, '-o', out], {encoding: 'utf8', timeout: 30000});
    assert.equal(res.status, 0, res.stderr);
    const bytes = await readFile(out);
    assert.ok(WebAssembly.validate(bytes), body);
    const module = new WebAssembly.Module(bytes);
    const guest = new WebAssembly.Instance(module, {atto: {}});
    assert.equal(guest.exports.main(), expected, body);
    assert.equal(guest.exports.panic_code.value, 0, body);
  }
  i = 0;
  for (const [body, code] of fail) {
    const input = join(work, `fail-${i}.atto`);
    const out = join(work, `fail-${i++}.wasm`);
    await writeFile(input, `pub fnc main(): int { ${body} }\n`);
    const res = spawnSync(attoc, [input, '-o', out], {encoding: 'utf8', timeout: 30000});
    assert.equal(res.status, 1, body + ': expected rejection; ' + res.stderr);
    assert.ok(res.stderr.includes(code), body + ': missing ' + code + ': ' + res.stderr);
    await assert.rejects(stat(out), {code: 'ENOENT'});
  }
  console.log('PASS', pass.length, 'executed guest sync-task fixtures and', fail.length, 'no-artifact rejections');
} finally {
  await rm(work, {recursive: true, force: true});
}
