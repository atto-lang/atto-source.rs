// Executed by CI only AFTER compiling the real source-compiler Rust binary.
// Uses Node's native WebAssembly engine: no guest interpreter or JIT fallback
// in the Atto compiler. The host engine itself executes Wasm per its runtime.
import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, writeFileSync, existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const compiler = resolve(process.argv[2] ?? 'target/debug/attoc');
const work = mkdtempSync(join(tmpdir(), 'atto-contracts-'));
let count = 0;
function compile(source, expectedError = null) {
  const base = join(work, String(++count));
  const input = base + '.atto', output = base + '.wasm';
  writeFileSync(input, source, 'utf8');
  const run = spawnSync(compiler, [input, '-o', output], { encoding: 'utf8' });
  if (expectedError !== null) {
    assert.notEqual(run.status, 0, `negative compile wrongly succeeded: ${source}`);
    assert.match(run.stderr, new RegExp(expectedError), `wrong compiler error: ${run.stderr}`);
    assert.equal(existsSync(output), false, 'invalid source emitted a guest artifact');
    return null;
  }
  assert.equal(run.status, 0, `real compiler failed: ${run.stderr}`);
  const bytes = readFileSync(output);
  assert.equal(bytes.subarray(0, 4).toString('hex'), '0061736d');
  assert.ok(WebAssembly.validate(bytes), 'real Atto compiler generated invalid guest Wasm');
  return new WebAssembly.Instance(new WebAssembly.Module(bytes), { atto: {} });
}
function success(source, expected) {
  const guest = compile(source);
  assert.equal(guest.exports.main(), BigInt(expected));
  assert.equal(guest.exports.panic_code.value, 0);
}
function failure(source, panicCode) {
  const guest = compile(source);
  assert.throws(() => guest.exports.main(), WebAssembly.RuntimeError);
  assert.equal(guest.exports.panic_code.value, panicCode);
}
try {
  // The callee evaluates the precondition before executing its body.
  success('fnc checked(int x) -> int requires x > 0; ensures |r| -> r > 0; { return x; } fnc main() -> int { return checked(42); }', 42);
  failure('fnc checked(int x) -> int requires x > 0; { return 99; } fnc main() -> int { return checked(0); }', 7);
  // The postcondition is checked after evaluating the actual return value.
  failure('fnc bad() -> int ensures |result| -> result > 0; { return 0; } fnc main() -> int { return bad(); }', 8);
  success('fnc fine() -> int ensures |r| -> r eq 42; { return 42; } fnc main() -> int { return fine(); }', 42);
  // All explicit normal return edges must check postconditions.
  failure('fnc choose(int x) -> int ensures |r| -> r > 0; { if x eq 0 then return 7; fi return 0; } fnc main() -> int { return choose(1); }', 8);
  success('fnc choose(int x) -> int ensures |r| -> r > 0; { if x eq 0 then return 7; fi return 0; } fnc main() -> int { return choose(0); }', 7);
  // bool predicates must retain the native i32/bool ABI.
  success('fnc choose(bool valid) -> bool requires valid; ensures |r| -> r; { return valid; } fnc main() -> int { if choose(true) then return 42; fi return 0; }', 42);
  failure('fnc choose(bool valid) -> bool requires valid; { return valid; } fnc main() -> int { if choose(false) then return 42; fi return 0; }', 7);
  // An ordinary caller-side trap must not catch a contract panic.
  failure('fnc bad(int x) -> int requires x > 0; { return x; } fnc main() -> int { val attempt = trap bad(0); return 42; }', 7);
  // Only statically proven constant true is removed at compile time.
  success('fnc main() -> int requires true; ensures |r| -> !false; { return 42; }', 42);
  // Never treat an effectful or unsupported predicate as safe to elide.
  compile('fnc positive(int x) -> bool { return x > 0; } fnc foo(int x) -> int requires positive(x); { return x; } fnc main() -> int { return foo(42); }', 'E_CONTRACT_EFFECT');
  compile('fnc foo(int x) -> int requires x + 1 > 0; { return x; } fnc main() -> int { return foo(42); }', 'E_CONTRACT_EFFECT');
  compile('fnc foo() -> int ensures |r| -> r; { return 42; } fnc main() -> int { return foo(); }', 'E_CONTRACT_NOT_BOOL');
  compile('fnc main() -> int requires false; { return 42; }', 'E_CONTRACT_ALWAYS_FALSE');
  console.log('PASS: 14 source-compiled Atto/Wasm positive, negative and panic-ABI assertions');
} finally {
  rmSync(work, { recursive: true, force: true });
}
