// Runtime oracles for real native-attoc / wasm32 compiler smoke fixtures.
// These checks cannot prove compiler provenance; the enclosing CI builds both
// compilers from the exact checked-out Rust source tree before calling them.
import assert from 'node:assert/strict';

const MAX_GUEST_BYTES = 1 << 20;

function instantiateScalarGuest(bytes) {
  assert.ok(bytes instanceof Uint8Array, 'compiled guest must be Wasm bytes');
  assert.ok(bytes.byteLength >= 8 && bytes.byteLength <= MAX_GUEST_BYTES,
    'compiled guest exceeds bounded Wasm size');
  assert.ok(WebAssembly.validate(bytes), 'compiled guest is not valid WebAssembly');
  const module = new WebAssembly.Module(bytes);
  assert.deepEqual(WebAssembly.Module.imports(module), [],
    'scalar guest must not access host imports');
  const declared = new Map(WebAssembly.Module.exports(module).map(item => [item.name, item.kind]));
  assert.equal(declared.get('main'), 'function', 'guest main export must be a function');
  assert.equal(declared.get('panic_code'), 'global', 'guest panic_code export must be a global');
  assert.equal(declared.get('memory'), 'memory', 'guest memory export must be a memory');
  const exports = new WebAssembly.Instance(module, {}).exports;
  assert.ok(exports.panic_code instanceof WebAssembly.Global, 'panic_code global missing');
  assert.ok(exports.memory instanceof WebAssembly.Memory, 'guest memory missing');
  assert.equal(exports.panic_code.value, 0, 'panic_code must initialize to zero');
  return exports;
}

export function verifyScalarGuestValue(bytes, expected) {
  assert.equal(typeof expected, 'bigint', 'the expected value must retain signed i64 precision');
  const exports = instantiateScalarGuest(bytes);
  const observed = exports.main();
  assert.equal(typeof observed, 'bigint', 'main must return a signed Wasm i64');
  assert.equal(exports.panic_code.value, 0, 'successful main must not set panic_code');
  assert.equal(observed, expected, 'unexpected compiled guest return value');
  return observed;
}

export function verifyCheckedPanicGuest(bytes, expectedCode) {
  assert.ok(Number.isSafeInteger(expectedCode) && expectedCode > 0,
    'language panic code must be an explicit positive integer');
  const exports = instantiateScalarGuest(bytes);
  assert.throws(() => exports.main(), WebAssembly.RuntimeError,
    'checked arithmetic must trap from a real Wasm guest');
  assert.equal(exports.panic_code.value, expectedCode,
    'trapped guest must expose the exact language panic code');
  return expectedCode;
}
