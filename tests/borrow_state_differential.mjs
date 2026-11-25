// Executed exact-source native/browser compiler regression for state loan
// isolation. Unlike host-only Rust tests, this asserts browser diagnostics,
// zero browser guest bytes and zero native stdout for rejected programs.
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {spawnSync} from 'node:child_process';
import {join} from 'node:path';
import {createStateRuntime} from './browser/state-runtime.mjs';

const wasmPath = process.argv[2] || 'target/wasm32-unknown-unknown/release/atto_compiler.wasm';
const nativePath = process.env.ATTOC || 'target/release/attoc';
const api = new WebAssembly.Instance(new WebAssembly.Module(await readFile(wasmPath)), {}).exports;
assert.equal(api.abi_version(), 2);
const enc = new TextEncoder(), dec = new TextDecoder();

function wasmCompile(src) {
  const bytes = enc.encode(src);
  assert.ok(bytes.length < api.input_capacity());
  new Uint8Array(api.memory.buffer, api.input_ptr(), bytes.length).set(bytes);
  const status = api.compile(bytes.length);
  if (status === 0) {
    const wasm = Buffer.from(new Uint8Array(api.memory.buffer, api.result_ptr(), api.result_length()));
    assert.deepEqual([...wasm.subarray(0, 8)], [0, 97, 115, 109, 1, 0, 0, 0]);
    return {ok: true, wasm};
  }
  assert.equal(status, 1);
  assert.equal(api.result_length(), 0, 'rejected guest must produce zero bytes');
  const mem = new Uint8Array(api.memory.buffer);
  let end = api.error_ptr();
  while (end < mem.length && mem[end]) ++end;
  assert.ok(end < mem.length);
  const message = dec.decode(mem.slice(api.error_ptr(), end));
  return {ok: false, code: message.split(':')[0]};
}

const cases = [
  ['pass', '196-state-owned-critical-write-preserved.atto'],
  ['fail', '210-state-mutable-alias-unleased-write.atto'],
  ['fail', '211-state-borrowed-receiver-method-write.atto'],
  ['fail', '212-state-outer-read-loan-mutating-argument.atto'],
];
for (const [kind, name] of cases) {
  const filename = join('tests/fixtures', kind, name);
  const source = await readFile(filename, 'utf8');
  const wasm = wasmCompile(source);
  const native = spawnSync(nativePath, [filename], {encoding: null, maxBuffer: 8 * 1024 * 1024});
  assert.equal(native.error, undefined, name);
  if (kind === 'fail') {
    const code = source.match(/^\/\/ expect-error: (\w+)$/m)?.[1];
    assert.ok(code, name);
    assert.equal(wasm.ok, false, name);
    assert.equal(wasm.code, code, name + ': browser diagnostic');
    assert.equal(native.status, 1, name + ': native admission');
    assert.equal(native.stdout.length, 0, name + ': native artifact bytes');
    assert.ok(native.stderr.toString().includes(code), name + ': native diagnostic');
  } else {
    const expectedExit = BigInt(source.match(/^\/\/ expect-exit: (\d+)$/m)?.[1]);
    assert.equal(wasm.ok, true, name);
    assert.equal(native.status, 0, name + ': native compiler');
    assert.deepEqual(wasm.wasm, native.stdout, name + ': compiler guest byte parity');
    const state = createStateRuntime();
    const imports = {atto: {
      print_i64: ()=>{}, print_bool: ()=>{}, print_str: ()=>{},
      print_newline: ()=>{}, print_f64: ()=>{},
      actor_spawn: ()=>{ throw Error('actor spawned in state loan test'); },
      actor_send: ()=>{ throw Error('actor send in state loan test'); },
      ...state.imports,
    }};
    try {
      const guest = new WebAssembly.Instance(new WebAssembly.Module(wasm.wasm), imports);
      const value = guest.exports.main();
      state.complete(); // no state tickets or authority may escape guest completion
      assert.equal(value, expectedExit, name + ': executed guest result');
    } catch (error) {
      state.abort();
      throw error;
    }
  }
}
console.log('PASS state-borrow differential: 1 executed positive, 3 fail-closed negatives, 4 native/browser decisions');
