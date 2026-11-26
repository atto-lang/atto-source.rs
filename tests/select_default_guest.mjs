// Execute exact source through native and Wasm-compiled Atto compilers.
import assert from 'node:assert/strict';
import { mkdtemp, writeFile, readFile, rm, access } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
const attoc = process.env.ATTOC || 'target/release/attoc';
const compiler = await readFile('target/wasm32-unknown-unknown/release/atto_compiler.wasm');
const { instance: { exports: api } } = await WebAssembly.instantiate(compiler, {});
assert.equal(api.abi_version(), 2);
function browser(source) {
  const b = new TextEncoder().encode(source);
  assert.ok(b.length <= api.input_capacity());
  new Uint8Array(api.memory.buffer, api.input_ptr(), b.length).set(b);
  const status = api.compile(b.length);
  if (status !== 0) { assert.equal(api.result_length(), 0); return null; }
  return Buffer.from(new Uint8Array(api.memory.buffer, api.result_ptr(), api.result_length()));
}
const positive = [
  'do select { default: -> { return 42; } } return 99;',
  'do select first over default: { return 42; } end return 99;',
  'let n: int = 40; do select fair { default: { n = n + 2; } } return n;',
  'do select random over default: { do select { default: { return 42; } } } end return 99;',
  'let n: int = 0; loop { do select { default: { n = n + 1; if n < 42 then continue; fi break; } } } return n;',
];
const negative = [
  'do select {} return 0;',
  'return select(3) { default: -> { return 42; } };',
  'return select<int>(3) over default: -> { return 42; } end;',
  'return await select<int>(3) { default: -> { return 42; } };',
  'return nb select(3) over default: -> { return 42; } end;',
  'do select(3) { default: {} } return 0;',
  'return try select(0) { default: -> { return 42; } };',
  'do select { default: -> -> {} } return 0;',
  'do select { default: {} default: {} } return 0;',
  'do select { when readch ch: val v {} } return 0;',
  'do nb select { default: {} } return 0;',
  'val x = select { default: { return 42; } }; return x;',
  'do select { default: { val hidden = 42; } } return hidden;',
  'do select { default: { return true; } } return 0;',
  'do select { default: { break; } } return 0;',
];
const dir = await mkdtemp(join(tmpdir(), 'atto-select-'));
try {
  let i = 0;
  for (const [bodies, ok] of [[positive, true], [negative, false]]) {
    for (const body of bodies) {
      const source = 'pub fnc main(): int { ' + body + ' }';
      const src = join(dir, String(i) + '.atto');
      const bin = join(dir, String(i++) + '.wasm');
      await writeFile(src, source);
      const r = spawnSync(attoc, [src, '-o', bin], { encoding: 'utf8', timeout: 30000 });
      assert.ifError(r.error);
      const guest = browser(source);
      if (ok) {
        assert.equal(r.status, 0, r.stderr);
        assert.ok(guest, source);
        assert.deepEqual(guest, await readFile(bin));
        const { instance } = await WebAssembly.instantiate(guest, { atto: {} });
        assert.equal(instance.exports.main(), 42n, source);
        assert.equal(instance.exports.panic_code.value, 0, source);
      } else {
        assert.ok(Number.isInteger(r.status) && r.status !== 0, source);
        assert.equal(guest, null, source);
        await assert.rejects(access(bin), { code: 'ENOENT' });
      }
    }
  }
  // Exact conformance #608 fixture bytes, not synthesized equivalents.
  // Six positive language targets are unsupported here, not parity passes.
  const countedNames = ["counted-three","counted-zero","counted-one","counted-alias","reject-counted-negative","reject-counted-bool","reject-counted-scalar","reject-counted-default","reject-counted-write","counted-dynamic","counted-dynamic-zero","reject-counted-dynamic-bool","reject-counted-dynamic-unknown"];
  for (const name of countedNames) {
    const src = 'tests/fixtures/select-source-v2/' + name + '.atto';
    const source = await readFile(src, 'utf8');
    const bin = join(dir, 'counted-' + name + '.wasm');
    const r = spawnSync(attoc, [src, '-o', bin], { encoding: 'utf8', timeout: 30000 });
    assert.ifError(r.error);
    assert.ok(Number.isInteger(r.status) && r.status !== 0, name + ': native unexpectedly emitted guest');
    assert.equal(browser(source), null, name + ': browser unexpectedly emitted guest');
    await assert.rejects(access(bin), {code: 'ENOENT'});
  }
  console.log('COUNTED_SELECT_EXACT_ADMISSION_PASS 13 native and 13 browser no-artifact rejections; 6 positive targets unsupported');
  console.log('SELECT_DEFAULT_GUEST_PASS 5 native/wasm32 byte-identical executions; 15 negative no-artifact cases');
} finally { await rm(dir, { recursive: true, force: true }); }
