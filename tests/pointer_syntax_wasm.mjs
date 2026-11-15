// Actual wasm32 compiler ABI + native attoc parity for Pointer<T> admission.
// This is a safety/negative gate, not a claim of executable pointers.
import assert from 'node:assert/strict';
import { readFileSync, writeFileSync, mkdtempSync, rmSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { join, resolve } from 'node:path';
import { tmpdir } from 'node:os';

const root = resolve(process.cwd());
const wasmBytes = readFileSync(join(root, 'target/wasm32-unknown-unknown/release/atto_compiler.wasm'));
const nativeAttoc = join(root, 'target/release/attoc');
const module = new WebAssembly.Module(wasmBytes);
assert.deepEqual(WebAssembly.Module.imports(module), [], 'compiler must be closed-world');
const c = new WebAssembly.Instance(module, {}).exports;
assert.equal(c.abi_version(), 2);
const utf8 = new TextEncoder();
const decode = new TextDecoder();
function compile(source) {
  const input = utf8.encode(source);
  assert.ok(input.length < c.input_capacity(), 'source must fit compiler buffer');
  new Uint8Array(c.memory.buffer, c.input_ptr(), input.length).set(input);
  const status = c.compile(input.length);
  if (status !== 0) {
    assert.equal(c.result_length(), 0, 'failed compile must publish no guest bytes');
    const mem = new Uint8Array(c.memory.buffer);
    const start = c.error_ptr();
    let end = start;
    while (end < start + 1024 && mem[end] !== 0) end++;
    return { ok: false, error: decode.decode(mem.subarray(start, end)) };
  }
  const bytes = new Uint8Array(c.memory.buffer, c.result_ptr(), c.result_length()).slice();
  assert.ok(WebAssembly.validate(bytes), 'compiled guest must be valid Wasm');
  return { ok: true, bytes };
}
const positive = 'pub fnc main(): int { return 6 * 7; }';
const good = compile(positive);
assert.equal(good.ok, true, good.error);
const guest = new WebAssembly.Instance(new WebAssembly.Module(good.bytes), {
  atto: { print_i64() {}, print_newline() {} }
});
assert.equal(guest.exports.main(), 42n);
const dir = mkdtempSync(join(tmpdir(), 'atto-pointer-abi-'));
try {
  const source = join(dir, 'arithmetic.atto');
  const output = join(dir, 'arithmetic.wasm');
  writeFileSync(source, positive);
  const native = spawnSync(nativeAttoc, [source, '-o', output], { encoding: 'utf8' });
  assert.equal(native.status, 0, native.stderr);
  assert.deepEqual(Buffer.from(good.bytes), readFileSync(output), 'native compiler and Wasm compiler must emit identical guest bytes');
} finally {
  rmSync(dir, { recursive: true, force: true });
}
const negatives = [
  ['pointer_type', 'pub fnc main(): int { val p: Pointer<int> = 0; return 0; }', 'E_UNSUPPORTED_TYPE'],
  ['pointer_return', 'pub fnc main(): Pointer<int> { return 0; }', 'E_UNSUPPORTED_TYPE'],
  ['pointer_class_shadow', 'define class Pointer as end\npub fnc main(): int { return 0; }', 'E_RESERVED_TYPE'],
  ['address_of', 'pub fnc main(): int { val x = 1; return &x; }', 'E_POINTER_SYNTAX'],
  ['unary_deref', 'pub fnc main(): int { val x = 1; return *x; }', 'E_POINTER_SYNTAX']
];
for (const [name, source, code] of negatives) {
  const outcome = compile(source);
  assert.equal(outcome.ok, false, name + ' unexpectedly compiled');
  assert.ok(outcome.error.includes(code), name + ': ' + outcome.error);
  console.log('POINTER_GUEST_NO_ARTIFACT=' + name + ' code=' + code);
}
console.log('PASS compiled Wasm multiplication guest exit=42, native-vs-Wasm byte parity, and 5 pointer no-artifact failures');
