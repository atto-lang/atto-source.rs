// Actual Rust-to-Wasm compiler and guest execution; not JS source evaluation.
// Requires the exact compiler built by this checkout and a native attoc binary.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';

const wasm = readFileSync('target/wasm32-unknown-unknown/release/atto_compiler.wasm');
assert.ok(WebAssembly.validate(wasm), 'actual Wasm compiler artifact');
const module = new WebAssembly.Module(wasm);
assert.deepEqual(WebAssembly.Module.imports(module), [], 'Wasm compiler is isolated');
const api = new WebAssembly.Instance(module, {}).exports;
assert.equal(api.abi_version(), 2);
const encoder = new TextEncoder();
const decoder = new TextDecoder('utf8', { fatal: true });

function compile(src) {
  const text = encoder.encode(src);
  assert.ok(text.length <= api.input_capacity(), 'source within ABI cap');
  new Uint8Array(api.memory.buffer, api.input_ptr(), text.length).set(text);
  const rc = api.compile(text.length);
  if (rc !== 0) {
    assert.equal(api.result_length(), 0, 'rejected source must not publish Wasm');
    const memory = new Uint8Array(api.memory.buffer);
    let end = api.error_ptr();
    while (memory[end]) end++;
    return {
      ok: false,
      error: decoder.decode(memory.subarray(api.error_ptr(), end)),
      line: api.error_line_number(),
      col: api.error_column_number()
    };
  }
  const guest = new Uint8Array(api.memory.buffer, api.result_ptr(), api.result_length()).slice();
  assert.ok(WebAssembly.validate(guest));
  return {ok: true, guest};
}
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
// Inspect the actual emitted Wasm section inventory rather than relying on
// a coincidental 0x11 byte in a function body or JS-simulated execution.
function section_ids(bytes) {
  const ids = new Set();
  let i = 8;
  while (i < bytes.length) {
    const id = bytes[i++];
    let size = 0, shift = 0, byte;
    do {
      assert.ok(i < bytes.length && shift < 35, 'valid canonical uleb section header');
      byte = bytes[i++];
      size += (byte & 127) * 2 ** shift;
      shift += 7;
    } while (byte & 128);
    assert.ok(i + size <= bytes.length, 'section length within Wasm');
    ids.add(id);
    i += size;
  }
  assert.equal(i, bytes.length, 'complete Wasm section parse');
  return ids;
}

const passes = [
  ['lambda-direct-capture-val', 42n],
  ['lambda-direct-capture-param', 42n],
  ['closure-lambda-functor-direct-profile', 42n],
  ['callable-owned-make-adder', 42n],
  ['callable-owned-apply-twice', 40n],
  ['callable-owned-independent-captures', 43n],
  ['callable-owned-multiple-targets', 41n],
  ['callable-owned-unrelated-control-flow', 42n]
];
for (const [name, expected] of passes) {
  const path = 'tests/fixtures/pass/' + name + '.atto';
  const source = readFileSync(path, 'utf8');
  const got = compile(source);
  assert.equal(got.ok, true, name + ': ' + got.error);
  const native = execFileSync('target/release/attoc', [path], {maxBuffer: 1024 * 1024});
  assert.equal(hash(got.guest), hash(native), name + ': exact native / wasm32 compiler byte parity');
  const sections = section_ids(got.guest);
  if (name.startsWith('callable-owned-')) {
    assert.ok(sections.has(4), name + ': must define a Wasm function table');
    assert.ok(sections.has(9), name + ': must initialize typed indirect call target');
  } else {
    assert.ok(!sections.has(4), name + ': direct-only module must have no callable table');
  }
  const guestModule = new WebAssembly.Module(got.guest);
  assert.deepEqual(WebAssembly.Module.imports(guestModule), [], 'no guest host imports');
  assert.equal(new WebAssembly.Instance(guestModule, {}).exports.main(), expected, name);
  console.log('PASS ' + name + ' actual guest ' + expected + ', native/wasm artifact SHA ' + hash(got.guest));
}
for (const [name, code] of [
  ['lambda-direct-capture-mutable', 'E_CAPTURE'],
  ['lambda-direct-capture-shadow', 'E_CAPTURE'],
  ['lambda-direct-capture-escape', 'E_UNSUPPORTED'],
  ['callable-old-skinny-arrow-type', 'E_SYNTAX'],
  ['callable-owned-mutable-capture', 'E_CAPTURE'],
  ['callable-owned-use-after-move', 'E_MOVED']
]) {
  const source = readFileSync('tests/fixtures/fail/' + name + '.atto', 'utf8');
  const got = compile(source);
  assert.equal(got.ok, false, name + ': must not emit a guest');
  assert.ok(got.error.startsWith(code + ':'), name + ': ' + got.error);
  assert.ok(got.line >= 1 && got.col >= 1, name + ': diagnostic position');
  // The same Wasm compiler instance must remain healthy after the rejection.
  const recovered = compile('pub fnc main(): int {return 42;}');
  assert.equal(recovered.ok, true, name + ': compiler state recovery');
}
console.log('PASS restricted closure/lambda/functor Rust & wasm32 compilation and fail-closed boundary');
