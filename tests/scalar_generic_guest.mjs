// Compile real scalar generic Atto source to Wasm and execute each guest.
import assert from 'node:assert/strict';
import { mkdtemp, writeFile, readFile, stat, rm } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const attoc = process.env.ATTOC || fileURLToPath(new URL('../target/debug/attoc', import.meta.url));
const declaration = 'fnc calculate<T>(int x): T { return (x + 3) as T; }\n';
const cases = [
  ['pub fnc main(): int { val x = calculate(5) as float; if x eq 8.0 then return 42; fi return 0; }', 42n],
  ['pub fnc main(): int { val x = calculate(5) : int; return x; }', 8n],
  ['pub fnc main(): int { val x: float = calculate(5); if x eq 8.0 then return 42; fi return 0; }', 42n],
  ['pub fnc main(): int { val x = calculate<float>(5); if x eq 8.0 then return 42; fi return 0; }', 42n],
  ['pub fnc main(): int { val x = calculate<int>(5); return x; }', 8n],
  ['pub fnc main(): int { val a = calculate<int>(5); val b = calculate<float>(5); if b eq 8.0 then return a; fi return 0; }', 8n],
];
const rejects = [
  ['pub fnc main(): int { val x = calculate(5); return 0; }', 'E_INFER'],
  ['pub fnc main(): int { val x = calculate<float>(5) : int; return 0; }', 'E_TYPE'],
  ['pub fnc main(): int { val x = calculate<bool>(5); return 0; }', 'E_UNSUPPORTED_CAST'],
];
const folder = await mkdtemp(join(tmpdir(), 'atto-scalar-gen-'));
try {
  for (const [index, [main, expected]] of cases.entries()) {
    const sourcePath = join(folder, 'case-' + index + '.atto');
    const wasmPath = join(folder, 'case-' + index + '.wasm');
    await writeFile(sourcePath, declaration + main + '\n');
    const built = spawnSync(attoc, [sourcePath, '-o', wasmPath],
      { encoding: 'utf8', timeout: 30000 });
    assert.equal(built.status, 0, main + ': ' + built.stderr);
    const wasm = await readFile(wasmPath);
    assert.ok(WebAssembly.validate(wasm), main);
    const { instance } = await WebAssembly.instantiate(wasm, { atto: {} });
    assert.equal(instance.exports.main(), expected, main);
    assert.equal(instance.exports.panic_code.value, 0, main);
  }
  for (const [index, [main, code]] of rejects.entries()) {
    const sourcePath = join(folder, 'reject-' + index + '.atto');
    const wasmPath = join(folder, 'reject-' + index + '.wasm');
    await writeFile(sourcePath, declaration + main + '\n');
    const built = spawnSync(attoc, [sourcePath, '-o', wasmPath],
      { encoding: 'utf8', timeout: 30000 });
    assert.notEqual(built.status, 0, main);
    assert.ok(built.stderr.includes(code), main + ': ' + built.stderr);
    await assert.rejects(stat(wasmPath), { code: 'ENOENT' });
  }
  console.log('PASS ' + cases.length + ' guest generic specializations; ' +
    rejects.length + ' fail-closed diagnostics');
} finally {
  await rm(folder, { recursive: true, force: true });
}
