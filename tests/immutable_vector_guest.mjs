// Real native/browser compiler decisions, Wasm bytes and executed guest values.
// This is a bounded immutable scalar Vector milestone, not general persistence.
import assert from 'node:assert/strict';
import { mkdtemp, readFile, writeFile, rm } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const native = process.env.ATTOC || 'target/release/attoc';
const wasm = process.env.ATTO_WASM || 'target/wasm32-unknown-unknown/release/atto_compiler.wasm';
const compiler = new WebAssembly.Instance(new WebAssembly.Module(await readFile(wasm)), {}).exports;
assert.equal(compiler.abi_version(), 2);
const encoder = new TextEncoder(), decoder = new TextDecoder();

function browser(source) {
  const bytes = encoder.encode(source);
  assert.ok(bytes.length < compiler.input_capacity());
  new Uint8Array(compiler.memory.buffer, compiler.input_ptr(), bytes.length).set(bytes);
  const status = compiler.compile(bytes.length);
  if (status === 0) {
    const output = new Uint8Array(compiler.memory.buffer, compiler.result_ptr(), compiler.result_length()).slice();
    assert.deepEqual([...output.slice(0, 8)], [0,97,115,109,1,0,0,0]);
    return { ok: true, bytes: output };
  }
  assert.equal(status, 1);
  assert.equal(compiler.result_length(), 0);
  const mem = new Uint8Array(compiler.memory.buffer);
  let end = compiler.error_ptr();
  while (end < mem.length && mem[end] !== 0) end++;
  assert.ok(end < mem.length);
  return { ok: false, error: decoder.decode(mem.slice(compiler.error_ptr(), end)) };
}

function execute(bytes) {
  let instance, stdout = '';
  const host = {
    print_i64(n) { stdout += String(n); },
    print_bool(n) { stdout += n ? 'true' : 'false'; },
    print_f64(n) { stdout += String(n); },
    print_str(ptr, len) { stdout += decoder.decode(new Uint8Array(instance.exports.memory.buffer, ptr, len)); },
    print_newline() { stdout += '\n'; },
    actor_spawn() { throw new Error('unadmitted actor spawn'); },
    actor_send() { throw new Error('unadmitted actor send'); }
  };
  const module = new WebAssembly.Module(bytes);
  for (const imp of WebAssembly.Module.imports(module)) {
    assert.equal(imp.module, 'atto');
    assert.ok(Object.hasOwn(host, imp.name));
  }
  instance = new WebAssembly.Instance(module, { atto: host });
  return { result: instance.exports.main(), stdout };
}

const positives = [
  ['pub fnc main(): int {const Vector<int> foo=[];return 42;}', 42n],
  ['pub fnc main(): int {const Vector<int> foo=[40,2];return foo[0]+foo[1];}', 42n],
  ['pub fnc main(): int {val Vector<int> foo=[10,32];return foo[0]+foo[1];}', 42n],
  ['pub fnc main(): int {val vector<int> foo=[42];return foo[0];}', 42n]
];
const negatives = [
  ['pub fnc main(): int {val int x=42;const Vector<int> foo=[x];return 0;}', 'E_NON_CONST_INITIALIZER'],
  ['pub fnc main(): int {val Vector<int> foo=[42];val x=borrow! mut foo;return 0;}', 'E_BORROW'],
  ['pub fnc main(): int {val Vector<int> foo=[42];return foo[1];}', 'E_INDEX_BOUNDS'],
  ['pub fnc main(): int {val Vector<int> foo=[42];val int i=0;return foo[i];}', 'E_UNSUPPORTED'],
  ['pub fnc main(): int {const Vector<int> foo=[1,\"bad\"];return 0;}', 'E_TYPE'],
  ['pub fnc main(): int {const Vector<int> foo=Array.of(1);return 0;}', 'E_NON_CONST_INITIALIZER']
];

const dir = await mkdtemp(join(tmpdir(), 'atto-immutable-vector-'));
try {
  let count = 0;
  for (const [source, value] of positives) {
    const file = join(dir, 'pass-'+count+++'.atto');
    await writeFile(file, source);
    const raw = spawnSync(native, [file], { encoding: null, maxBuffer: 4*1024*1024 });
    assert.equal(raw.error, undefined);
    assert.equal(raw.status, 0, String(raw.stderr));
    const web = browser(source);
    assert.equal(web.ok, true, web.error);
    assert.deepEqual(raw.stdout, Buffer.from(web.bytes));
    assert.deepEqual(execute(web.bytes), { result: value, stdout: '' });
  }
  count = 0;
  for (const [source, diagnostic] of negatives) {
    const file = join(dir, 'reject-'+count+++'.atto');
    await writeFile(file, source);
    const raw = spawnSync(native, [file], { encoding: null, maxBuffer: 4*1024*1024 });
    assert.equal(raw.error, undefined);
    assert.equal(raw.status, 1, String(raw.stderr));
    assert.equal(raw.stdout.length, 0);
    assert.ok(String(raw.stderr).includes(diagnostic), String(raw.stderr));
    const web = browser(source);
    assert.equal(web.ok, false, 'browser compiler emitted invalid guest');
    assert.equal(web.error.split(':')[0], diagnostic);
  }
  console.log('PASS bounded immutable Vector: '+positives.length+' compiled & executed Wasm guests; '+negatives.length+' no-artifact negatives; native/browser compiler byte parity');
} finally {
  await rm(dir, { recursive: true, force: true });
}
