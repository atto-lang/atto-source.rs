// Real guest-Wasm execution for expression type ascription and checked-as.
import assert from 'node:assert/strict';
import { mkdtemp, writeFile, readFile, stat, rm } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const attoc = process.env.ATTOC || fileURLToPath(new URL('../target/debug/attoc', import.meta.url));
const positives = [
  ['return 42 : int;', 42n],
  ['return 42 as int;', 42n],
  ['return (40 + 2) : int;', 42n],
  ['return (40 + 2) as int;', 42n],
  ['return (true ? 42 : 7) as int;', 42n],
  ['return true ? (42 : int) : 7;', 42n],
  ['val x = (40 + 2) : int; return x;', 42n],
  ['val x = (40 + 2) as int; return x;', 42n],
];
const negatives = [
  ['return 42 : bool;', 'E_TYPE'],
  ['return 42 as bool;', 'E_UNSUPPORTED_CAST'],
  ['return 42 as float;', 'E_UNSUPPORTED_CAST'],
];
const work = await mkdtemp(join(tmpdir(), 'atto-ascription-'));
try {
  for (const [i, [body, expected]] of positives.entries()) {
    const source = join(work, `pass-${i}.atto`);
    const wasmPath = join(work, `pass-${i}.wasm`);
    await writeFile(source, `pub fnc main(): int { ${body} }\n`);
    const result = spawnSync(attoc, [source, '-o', wasmPath], { encoding: 'utf8', timeout: 30000 });
    assert.equal(result.status, 0, `${body}: ${result.stderr}`);
    const wasm = await readFile(wasmPath);
    assert.ok(WebAssembly.validate(wasm), body);
    const { instance } = await WebAssembly.instantiate(wasm, { atto: {} });
    assert.equal(instance.exports.main(), expected, body);
    assert.equal(instance.exports.panic_code.value, 0, body);
  }
  for (const [i, [body, code]] of negatives.entries()) {
    const source = join(work, `reject-${i}.atto`);
    const wasmPath = join(work, `reject-${i}.wasm`);
    await writeFile(source, `pub fnc main(): int { ${body} }\n`);
    const result = spawnSync(attoc, [source, '-o', wasmPath], { encoding: 'utf8', timeout: 30000 });
    assert.notEqual(result.status, 0, body);
    assert.ok(result.stderr.includes(code), `${body}: ${result.stderr}`);
    await assert.rejects(stat(wasmPath), { code: 'ENOENT' }, body);
  }
  console.log('PASS', positives.length, 'ascription/checked-as Wasm guest cases and',
    negatives.length, 'negative fail-closed cases');
} finally {
  await rm(work, { recursive: true, force: true });
}
