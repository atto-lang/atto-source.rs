import assert from 'node:assert/strict';
import test from 'node:test';
// Worker uses real ECMAScript modules. Give it a harmless self to register onmessage.
globalThis.self = {};
const {validateGuestImportABI} = await import('./worker.mjs');

const u32 = n => {
  const out = [];
  do { let b = n & 0x7f; n >>>= 7; if (n) b |= 0x80; out.push(b); } while (n);
  return out;
};
const name = s => { const b = [...new TextEncoder().encode(s)]; return [...u32(b.length), ...b]; };
const section = (id, body) => [id, ...u32(body.length), ...body];
const MAGIC = [0, 97, 115, 109, 1, 0, 0, 0];
function guest({mod='atto', symbol='print_i64', params=[0x7e], results=[], kind=0,
                duplicates=false, skipType=false, noncanonical=false} = {}) {
  const type = [1, 0x60, ...u32(params.length), ...params, ...u32(results.length), ...results];
  const one = [...name(mod), ...name(symbol), kind,
    ...(kind === 0 ? [0] : kind === 2 ? [0, 1] : [0])];
  const imports = [duplicates ? 2 : 1, ...one, ...(duplicates ? one : [])];
  const bytes = [...MAGIC, ...(!skipType ? section(1, type) : []), ...section(2, imports)];
  if (noncanonical) bytes.push(0, 0x80, 0x00);
  return Uint8Array.from(bytes);
}
function mustReject(bytes, pattern = /E_ABI:/) {
  assert.throws(() => validateGuestImportABI(bytes), pattern);
}

test('minimal no-import guest accepted', () => {
  const w = Uint8Array.from(MAGIC);
  assert.equal(WebAssembly.validate(w), true);
  assert.deepEqual(validateGuestImportABI(w, new WebAssembly.Module(w)), []);
});
test('legitimate exact print_i64 signature accepted', () => {
  const w = guest();
  assert.equal(WebAssembly.validate(w), true);
  assert.deepEqual(validateGuestImportABI(w, new WebAssembly.Module(w)),
    [{module:'atto', name:'print_i64', kind:'function'}]);
});
test('all currently approved host imports have exact types', () => {
  const cases = [
    ['print_bool', [0x7f], []], ['print_str', [0x7f, 0x7f], []],
    ['print_newline', [], []], ['print_f64', [0x7c], []],
    ['state_register', [0x7f], []], ['state_enter', [0x7f, 0x7f], [0x7e]],
    ['state_leave', [0x7e], []],
  ];
  for (const [symbol, params, results] of cases) {
    const w = guest({symbol, params, results});
    assert.equal(WebAssembly.validate(w), true, symbol);
    assert.equal(validateGuestImportABI(w, new WebAssembly.Module(w)).length, 1, symbol);
  }
});
test('foreign namespace and ungranted actor/callback imports reject', () => {
  mustReject(guest({mod:'env'}), /not granted/);
  mustReject(guest({symbol:'actor_spawn', params:[0x7f], results:[0x7f]}), /not granted/);
  mustReject(guest({symbol:'__proto__'}), /not granted/);
});
test('JS ignores function import signature but admission does not', () => {
  for (const override of [
    {params:[]}, {params:[0x7f]}, {params:[0x7c]}, {params:[0x7e,0x7e]},
    {results:[0x7e]},
  ]) {
    const w = guest(override);
    assert.equal(WebAssembly.validate(w), true);
    mustReject(w, /signature mismatch/);
  }
});
test('foreign memory import denied even with allowed name', () => {
  const w = guest({kind:2});
  assert.equal(WebAssembly.validate(w), true);
  mustReject(w, /non-function/);
});
test('duplicate import denied even though Wasm allows it', () => {
  const w = guest({duplicates:true});
  assert.equal(WebAssembly.validate(w), true);
  mustReject(w, /duplicate guest import/);
});
test('broken, reordered, noncanonical, oversized inputs fail closed', () => {
  mustReject(Uint8Array.of(), /byte length/);
  mustReject(Uint8Array.from([1,2,3,4,5,6,7,8]), /header/);
  mustReject(guest({skipType:true}), /signature mismatch/);
  mustReject(guest({noncanonical:true}), /noncanonical/);
  mustReject(guest().subarray(0, guest().length-1), /truncated/);
});
test('reflected native import metadata must match byte parser', () => {
  const w = guest();
  const fakeModule = {}; // native WebAssembly.Module.imports rejects non-module
  assert.throws(() => validateGuestImportABI(w, fakeModule));
});
