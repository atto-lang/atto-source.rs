// Real native/browser compiler + executed guest conformance for the opt-in
// borrow-v1 profile. No source-free oracle and no silent fallback to legacy.
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {spawnSync} from 'node:child_process';
import {join} from 'node:path';

const browser = process.argv[2] || 'target/wasm32-unknown-unknown/release/atto_compiler.wasm';
const native = process.env.ATTOC || 'target/release/attoc';
const wasm = new WebAssembly.Module(await readFile(browser));
assert.deepEqual(WebAssembly.Module.imports(wasm), [], 'standalone no-import browser compiler');
const api = new WebAssembly.Instance(wasm, {}).exports;
assert.equal(api.abi_version(), 2);
assert.equal(typeof api.compile_borrow_v1, 'function', 'explicit versioned wasm export');
const enc = new TextEncoder();
const dec = new TextDecoder();

function compileGuest(source, mode = 'borrow-v1') {
  const input = enc.encode(source);
  assert.ok(input.length < api.input_capacity(), 'bounded source');
  new Uint8Array(api.memory.buffer, api.input_ptr(), input.length).set(input);
  const status = mode === 'borrow-v1'
    ? api.compile_borrow_v1(input.length)
    : api.compile(input.length);
  assert.ok(status === 0 || status === 1, 'compiler success/diagnostic only');
  if (status === 0) {
    const code = new Uint8Array(api.memory.buffer, api.result_ptr(), api.result_length()).slice();
    assert.deepEqual([...code.slice(0, 8)], [0, 97, 115, 109, 1, 0, 0, 0]);
    return {ok: true, wasm: code};
  }
  assert.equal(api.result_length(), 0, 'no guest bytes on rejection');
  const mem = new Uint8Array(api.memory.buffer);
  const start = api.error_ptr();
  let end = start;
  while (end < mem.length && mem[end] !== 0) end++;
  assert.ok(end < mem.length, 'error zero-terminated');
  const diagnostic = dec.decode(mem.slice(start, end));
  return {ok: false, diagnostic, code: diagnostic.split(':')[0]};
}

function executeGuest(bytes) {
  let output = '';
  let instance;
  const imports = {
    print_i64: n => { output += String(n); },
    print_bool: n => { output += n ? 'true' : 'false'; },
    print_str: (p, len) => {
      output += dec.decode(new Uint8Array(instance.exports.memory.buffer, p, len));
    },
    print_newline: () => { output += '\n'; },
    print_f64: n => { output += String(n); },
    actor_spawn: () => { throw Error('actor spawn not admitted in borrow-v1 profile'); },
    actor_send: () => { throw Error('actor send not admitted in borrow-v1 profile'); }
  };
  instance = new WebAssembly.Instance(new WebAssembly.Module(bytes), {atto: imports});
  return {value: instance.exports.main(), output};
}

const pass = [
  '01-plain-read-preserves-owner.atto',
  '02-explicit-implicit-equivalence.atto',
  '03-two-readers-one-call.atto',
  '04-legacy-scalar-unchanged.atto',
  '05-explicit-owned-transfer.atto',
  '06-consuming-name-colon-syntax.atto',
];
const fail = [
  '01-borrowed-parameter-not-owned.atto',
  '02-reject-implicit-mutable-access.atto',
  '03-temporary-owner-not-proven.atto',
  '04-reject-take-to-readonly.atto',
  '05-returned-loan-not-implemented.atto',
  '06-method-class-formal-not-admitted.atto',
  '07-mutable-class-formal-needs-exclusive-abi.atto',
  '08-take-formal-requires-take-call.atto',
  '09-consumed-owner-is-moved.atto',
  '10-take-formal-no-temporary.atto',
  '11-take-formal-no-readonly-borrow.atto',
  '12-read-then-take-same-owner.atto',
  '13-take-then-read-same-owner.atto',
  '14-borrowed-formal-cannot-be-taken.atto',
];
const positiveBytes = [];
let decisions = 0;
for (const [kind, names] of [['pass', pass], ['fail', fail]]) {
  for (const name of names) {
    const path = join('tests/fixtures/borrow-v1', kind, name);
    const source = await readFile(path, 'utf8');
    const guest = compileGuest(source);
    const local = spawnSync(native, ['--borrow-v1', path], {encoding: null, maxBuffer: 8 * 1024 * 1024});
    assert.equal(local.error, undefined, name);
    if (kind === 'pass') {
      assert.equal(guest.ok, true, name + ': ' + guest.diagnostic);
      assert.equal(local.status, 0, name + ': native compiler ' + local.stderr);
      assert.deepEqual(local.stdout, Buffer.from(guest.wasm), name + ': byte-identical native/browser compilers');
      const expected = source.match(/^\/\/ expect-exit: (\d+)$/m);
      assert.ok(expected, name + ': required expected exit marker');
      const run = executeGuest(guest.wasm);
      assert.equal(run.value, BigInt(expected[1]), name + ': guest return');
      assert.equal(run.output, '', name + ': no hidden console effects');
      positiveBytes.push(guest.wasm);
    } else {
      const expected = source.match(/^\/\/ expect-error: (\w+)$/m);
      assert.ok(expected, name);
      assert.equal(guest.ok, false, name + ': invalid source emitted guest');
      assert.equal(guest.code, expected[1], name + ': browser rejection diagnostic');
      assert.equal(local.status, 1, name + ': native admitted invalid source');
      assert.equal(local.stdout.length, 0, name + ': native emitted guest on rejection');
      assert.ok(local.stderr.toString().includes(expected[1]), name + ': native diagnostic');
    }
    decisions++;
  }
}
// These programs differ only by whether the first read is explicitly marked.
// Their serialized guest Wasm must therefore agree byte-for-byte.
assert.deepEqual(positiveBytes[0], positiveBytes[1], 'implicit vs borrow! code generation');
// The old ABI/compiler entry point is unchanged and still consumes Person.
const oldSource = await readFile(join('tests/fixtures/borrow-v1/pass', pass[0]), 'utf8');
const oldBrowser = compileGuest(oldSource, 'legacy');
const oldNative = spawnSync(native, [join('tests/fixtures/borrow-v1/pass', pass[0])], {encoding: null});
assert.equal(oldBrowser.ok, false, 'legacy compiler not accidentally switched');
assert.equal(oldBrowser.code, 'E_MOVED');
assert.equal(oldNative.status, 1);
assert.equal(oldNative.stdout.length, 0);
assert.ok(oldNative.stderr.toString().includes('E_MOVED'));
console.log('PASS borrow-v1 guest: ' + pass.length +
  ' executed positive guests, ' + fail.length +
  ' zero-artifact negative diagnostics, ' + decisions +
  ' native/browser decisions + byte parity, default ABI unchanged');
