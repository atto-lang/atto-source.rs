// Prove the actual wasm32 compiler (not JS emulation or native-only lowering)
// compiles the exact website bitwise preview into executable guest Wasm.
import assert from 'node:assert/strict';
import { readFile, writeFile, mkdtemp, rm } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { join } from 'node:path';
import { tmpdir } from 'node:os';

const here = new URL('.', import.meta.url);
const sourcePath = fileURLToPath(new URL('./fixtures/pass/192-bitwise-playground.atto', here));
const compilerPath = fileURLToPath(new URL('../target/wasm32-unknown-unknown/release/atto_compiler.wasm', here));
const attoc = process.env.ATTOC || fileURLToPath(new URL('../target/debug/attoc', here));
const source = await readFile(sourcePath, 'utf8');
assert.match(source, /0b101100/, 'test must retain exact preview binary flags');
assert.match(source, />>>/, 'test must retain logical shift syntax');

const compilerBytes = await readFile(compilerPath);
assert.ok(WebAssembly.validate(compilerBytes), 'compiler itself must be valid Wasm');
const module = new WebAssembly.Module(compilerBytes);
assert.deepEqual(WebAssembly.Module.imports(module), [], 'no browser compiler host imports');
const { exports: api } = new WebAssembly.Instance(module, {});
assert.equal(api.abi_version(), 2, 'stable public browser compiler ABI');
const encoder = new TextEncoder();
const decoder = new TextDecoder('utf8', { fatal: true });

function compile(sourceText) {
  const bytes = encoder.encode(sourceText);
  assert.ok(bytes.length <= api.input_capacity());
  new Uint8Array(api.memory.buffer, api.input_ptr(), bytes.length).set(bytes);
  const status = api.compile(bytes.length);
  if (status !== 0) {
    assert.equal(api.result_length(), 0, 'failed compile may not return guest bytes');
    const memory = new Uint8Array(api.memory.buffer);
    const p = api.error_ptr();
    let e = p;
    while (memory[e] && e < p + 512) e++;
    return { ok: false, diagnostic: decoder.decode(memory.subarray(p, e)) };
  }
  const guest = new Uint8Array(api.memory.buffer, api.result_ptr(), api.result_length()).slice();
  assert.ok(guest.byteLength > 8);
  assert.ok(WebAssembly.validate(guest));
  return { ok: true, guest };
}

// Canonical operator spelling must execute through the real wasm32 compiler.
const caretXorGuest = compile('pub fnc main(): int { ^ 12 ^^ 10; }');
assert.equal(caretXorGuest.ok, true, caretXorGuest.diagnostic || 'caret/XOR compile');
const caretXorModule = new WebAssembly.Module(caretXorGuest.guest);
assert.deepEqual(WebAssembly.Module.imports(caretXorModule), [], 'caret/XOR guest has no imports');
const caretXorRuntime = new WebAssembly.Instance(caretXorModule, { atto: {} });
assert.equal(caretXorRuntime.exports.main(), 6n, 'return marker and XOR are unambiguous');
assert.equal(caretXorRuntime.exports.panic_code.value, 0);

// The same wasm32-hosted compiler must lower signed i64 division to f64,
// not silently truncate the quotient and not emit an int-returning guest.
const divisionSource = `fnc half(int numerator, int denominator): float {
  return numerator / denominator;
}
pub fnc main(): int {
  if half(5, 2) eq 2.5 then return 42; fi
  return 1;
}`;
const division = compile(divisionSource);
assert.equal(division.ok, true, division.diagnostic || 'int/int -> float compile');
const divisionModule = new WebAssembly.Module(division.guest);
assert.deepEqual(WebAssembly.Module.imports(divisionModule), []);
const divisionGuest = new WebAssembly.Instance(divisionModule, { atto: {} });
assert.equal(divisionGuest.exports.main(), 42n, 'integer division is floating');
assert.equal(divisionGuest.exports.panic_code.value, 0);

const rejectedTruncation = compile('pub fnc main(): int { return 5 / 2; }');
assert.equal(rejectedTruncation.ok, false, 'float result must not narrow to int');
assert.match(rejectedTruncation.diagnostic, /E_TYPE|E_RETURN_TYPE/);

const rejectedShift = compile('pub fnc main(): int { return 1 <<< 64; }');
assert.equal(rejectedShift.ok, false, 'invalid constant shift must not emit Wasm');
assert.match(rejectedShift.diagnostic, /E_SHIFT_DISTANCE/);

const runtimeShift = compile('pub fnc main(): int { let distance: int = 64; return 1 <<< distance; }');
assert.equal(runtimeShift.ok, true, runtimeShift.diagnostic || 'dynamic checked shift compile');
const checkedShift = new WebAssembly.Instance(new WebAssembly.Module(runtimeShift.guest), { atto: {} });
assert.throws(() => checkedShift.exports.main(), WebAssembly.RuntimeError);
assert.equal(checkedShift.exports.panic_code.value, 7, 'must not mask dynamic shift by 63');

const compiled = compile(source);
assert.equal(compiled.ok, true, compiled.diagnostic || 'preview must compile');
const guestModule = new WebAssembly.Module(compiled.guest);
assert.deepEqual(WebAssembly.Module.imports(guestModule), [], 'bitwise guest requires no imports');
const instance = new WebAssembly.Instance(guestModule, { atto: {} });
assert.equal(instance.exports.main(), 188n, 'exact website mask/shift result');
assert.equal(instance.exports.panic_code.value, 0, 'no guest panic');

const tmp = await mkdtemp(join(tmpdir(), 'atto-bitwise-browser-'));
try {
  const out = join(tmp, 'bitwise.wasm');
  const result = spawnSync(attoc, [sourcePath, '-o', out], { encoding: 'utf8', timeout: 30000 });
  assert.equal(result.status, 0, result.stderr);
  const nativeBytes = await readFile(out);
  assert.deepEqual(Buffer.from(compiled.guest), nativeBytes,
    'native and wasm32 compilers must emit byte-identical guest for exact preview');
  const floatSource = join(tmp, 'float-division.atto');
  const floatArtifact = join(tmp, 'float-division.wasm');
  await writeFile(floatSource, divisionSource + '\n');
  const nativeFloat = spawnSync(attoc, [floatSource, '-o', floatArtifact],
    { encoding: 'utf8', timeout: 30000 });
  assert.equal(nativeFloat.status, 0, nativeFloat.stderr);
  assert.deepEqual(await readFile(floatArtifact), Buffer.from(division.guest),
    'native and wasm32 compilers must produce byte-identical float division guests');

} finally {
  await rm(tmp, { recursive: true, force: true });
}

for (const [src, expected] of [
  ['pub fnc main(): int { return 0b2; }', 'E_INTEGER_LITERAL'],
  ['pub fnc main(): int { return 1 ^^ true; }', 'E_TYPE'],
  ['pub fnc main(): int { return 1 ^ 2; }', 'E_SEMICOLON'],
  ['pub fnc main(): int { return 0b; }', 'E_INTEGER_LITERAL'],
]) {
  const failure = compile(src);
  assert.equal(failure.ok, false, 'negative source cannot return a Wasm artifact');
  assert.ok(failure.diagnostic.includes(expected), failure.diagnostic);
}
console.log('PASS website bitwise Wasm guest, native bytes, floating division, checked shifts and source rejection');
