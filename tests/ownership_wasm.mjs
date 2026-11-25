// Exact browser compiler ABI regression: scalar ownership may execute,
// unsupported loans/refcounts/atomics must produce zero guest Wasm bytes.
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';

const path = process.argv[2] || 'target/wasm32-unknown-unknown/release/atto_compiler.wasm';
const bytes = await readFile(path);
const module = new WebAssembly.Module(bytes);
assert.deepEqual(WebAssembly.Module.imports(module), [], 'compiler must remain standalone');
const api = new WebAssembly.Instance(module, {}).exports;
assert.equal(api.abi_version(), 2);
const enc = new TextEncoder();
const dec = new TextDecoder();
function compile(source) {
  const input = enc.encode(source);
  assert.ok(input.length <= api.input_capacity());
  new Uint8Array(api.memory.buffer, api.input_ptr(), input.length).set(input);
  const status = api.compile(input.length);
  if (status === 0) {
    const out = new Uint8Array(api.memory.buffer, api.result_ptr(), api.result_length()).slice();
    assert.deepEqual([...out.slice(0, 8)], [0,97,115,109,1,0,0,0]);
    return {ok: true, out};
  }
  assert.equal(status, 1);
  assert.equal(api.result_length(), 0, 'diagnostic must erase guest output');
  const buffer = new Uint8Array(api.memory.buffer);
  const start = api.error_ptr();
  let end = start;
  while (end < buffer.length && buffer[end]) end++;
  assert.ok(end < buffer.length);
  const diagnostic = dec.decode(buffer.slice(start, end));
  return {ok: false, code: diagnostic.split(':')[0], diagnostic};
}
function accept(source) {
  const r = compile(source);
  assert.equal(r.ok, true, r.diagnostic || source);
}
function reject(source, code) {
  const r = compile(source);
  assert.equal(r.ok, false, source);
  assert.equal(r.code, code, r.diagnostic);
}
accept('pub fnc main(): int {val n=40; val c=copy! n; val t=take! n; return c+t;}');
accept('pub fnc main(): int {let n=5; val c=take! n; n=37; return c+copy! n;}');
reject('pub fnc main(): int {val n=42; val x=take! n; return n;}', 'E_MOVED');
reject('pub fnc main(): int {val n=42; val x=take! n; return take! n;}', 'E_MOVED');
reject('pub fnc main(): int {return take! (40+2);}', 'E_MOVE');
accept('pub fnc main(): int {let n=40; block {val a=borrow! n; val b=borrow! n; stdio.println(copy! a+copy! b);} n=42; return n;}');
accept('pub fnc main(): int {let n=40; block {val writer=borrow! mut n; writer=42;} return n;}');
reject('pub fnc main(): int {val n=42; val b=borrow! n; return b;}', 'E_ESCAPE');
reject('pub fnc main(): int {let n=42; val b=borrow! mut n; return b;}', 'E_ESCAPE');
reject('pub fnc main(): int {let n=42; val b=borrow! n; n=43; return 0;}', 'E_BORROW');
reject('pub fnc main(): int {let n=42; val b=borrow! mut n; return n;}', 'E_BORROW');
reject('pub fnc main(): int {let n=42; val a=borrow! n; val b=borrow! mut n; return 0;}', 'E_BORROW');
reject('pub fnc main(): int {let n=42; val a=borrow! mut n; val b=borrow! n; return 0;}', 'E_BORROW');
reject('pub fnc main(): int {let n=42; val a=borrow! mut n; a=41; return take! n;}', 'E_BORROW');
reject('pub fnc main(): int {val n=42; val b=share! n; return n;}', 'E_UNSUPPORTED');
for (const t of ['Box', 'Arc', 'Atomic']) {
  reject('pub fnc main(): int {val n: '+t+'<int> = 0; return 0;}', 'E_UNSUPPORTED_TYPE');
}
console.log('PASS browser Wasm ownership ABI: 4 scalar admits, 14 fail-closed cases');
