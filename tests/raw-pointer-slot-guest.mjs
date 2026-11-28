// Dedicated actual browser compiler and native-host CLI guest test.
// This does not replace the full corpus; its separate result isolates this profile.
import assert from 'node:assert/strict';
import { readFile, mkdtemp, rm, access } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { join, resolve } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';

const here = fileURLToPath(new URL('.', import.meta.url));
const compiler = resolve(process.argv[2] || join(here, '../target/wasm32-unknown-unknown/release/atto_compiler.wasm'));
const cli = resolve(process.env.ATTOC || join(here, '../target/release/attoc'));
const module = new WebAssembly.Module(await readFile(compiler));
assert.deepEqual(WebAssembly.Module.imports(module), []);
const { exports: api } = new WebAssembly.Instance(module);
assert.equal(api.abi_version(), 2);
const encoder = new TextEncoder(), decoder = new TextDecoder('utf-8', { fatal: true });
const work = await mkdtemp(join(tmpdir(), 'atto-raw-slot-'));
const absent = async path => {
  try { await access(path); assert.fail('unexpected output artifact: ' + path); }
  catch (e) { if (e.code !== 'ENOENT') throw e; }
};
function compile(source) {
  const bytes = encoder.encode(source);
  assert.ok(bytes.length <= api.input_capacity());
  new Uint8Array(api.memory.buffer, api.input_ptr(), bytes.length).set(bytes);
  const status = api.compile(bytes.length);
  if (status !== 0) {
    assert.equal(api.result_length(), 0);
    const memory = new Uint8Array(api.memory.buffer);
    const start = api.error_ptr(), end = memory.indexOf(0, start);
    assert.ok(end >= start);
    return { code: decoder.decode(memory.subarray(start, end)).split(':')[0] };
  }
  const wasm = new Uint8Array(api.memory.buffer, api.result_ptr(), api.result_length()).slice();
  assert.ok(WebAssembly.validate(wasm));
  return { wasm };
}
const positive = ['empty', 'rebind', 'survivor', 'budget-boundary'];
const negative = ['owner-budget', 'operation-budget', 'term-budget'];
const failIfCalled = () => { throw Error('pure scalar guest unexpectedly invoked host'); };
let passed = 0;
try {
  for (const name of positive) {
    const file = join(here, 'fixtures/pass/raw-pointer-slot-' + name + '.atto');
    const source = await readFile(file, 'utf8');
    const { wasm } = compile(source);
    assert.ok(wasm, name);
    const guest = new WebAssembly.Module(wasm);
    const imports = {};
    for (const item of WebAssembly.Module.imports(guest)) {
      assert.equal(item.module, 'atto');
      assert.equal(item.kind, 'function');
      (imports[item.module] ??= {})[item.name] = failIfCalled;
    }
    const result = new WebAssembly.Instance(guest, imports).exports.main();
    assert.ok(result === 42 || result === 42n, name + ': actual guest result');
    const output = join(work, name + '.wasm');
    const native = spawnSync(cli, [file, '-o', output], { encoding: 'utf8' });
    assert.ifError(native.error);
    assert.equal(native.status, 0, native.stderr);
    assert.deepEqual(new Uint8Array(await readFile(output)), wasm, name + ': native/browser byte parity');
    console.log('RAW_SLOT_GUEST_PASS ' + name + ' exit=42');
    passed++;
  }
  for (const name of negative) {
    const file = join(here, 'fixtures/fail/raw-pointer-slot-' + name + '.atto');
    const source = await readFile(file, 'utf8');
    assert.equal(compile(source).code, 'E_POINTER_SLOT_PROFILE', name);
    const output = join(work, name + '.wasm');
    await absent(output);
    const native = spawnSync(cli, [file, '-o', output], { encoding: 'utf8' });
    assert.ifError(native.error);
    assert.equal(native.signal, null);
    assert.ok(Number.isInteger(native.status) && native.status !== 0);
    assert.match(native.stderr, /(?:^|\s)E_POINTER_SLOT_PROFILE:/m, name);
    await absent(output);
    console.log('RAW_SLOT_REJECTION_PASS ' + name);
    passed++;
  }
  assert.equal(passed, 7);
  console.log('ATTO_RAW_SLOT_FOCUSED_GUEST_PASS positive=4 negative=3');
} finally { await rm(work, { recursive: true, force: true }); }
