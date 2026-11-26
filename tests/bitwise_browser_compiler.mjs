// Prove the actual wasm32 compiler (not JS emulation or native-only lowering)
// compiles the exact website bitwise preview into executable guest Wasm.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
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

// The same integer XOR compound spelling is admitted only for mutable locals.
const compoundXorSource = 'pub fnc main(): int { let x: int = 12; x ^^= 10; ^ x; }';
const compoundXor = compile(compoundXorSource);
assert.equal(compoundXor.ok, true, compoundXor.diagnostic);
const compoundXorGuest = new WebAssembly.Instance(new WebAssembly.Module(compoundXor.guest), {});
assert.equal(compoundXorGuest.exports.main(), 6n);
assert.equal(compoundXorGuest.exports.panic_code.value, 0);
for (const body of ['val x: int = 12; x ^^= 10; return x;',
                    'let x: int = 12; x ^^= true; return x;',
                    'let x: int = 12; x ^= 10; return x;',
                    'let x: int = 12; x ^^ = 10; return x;',
                    'let x: int = 12; x ^ ^= 10; return x;']) {
  assert.equal(compile(`pub fnc main(): int { ${body} }`).ok, false, body);
}

// Standalone statement-leading ^ is return, never a side-effect-only prefix.
const prefixReturn = compile('pub fnc main(): int { ^ 42; }');
assert.equal(prefixReturn.ok, true, prefixReturn.diagnostic || 'prefix caret return');
const prefixGuest = new WebAssembly.Instance(new WebAssembly.Module(prefixReturn.guest), { atto: {} });
assert.equal(prefixGuest.exports.main(), 42n, 'prefix caret must exit the callable');
assert.equal(prefixGuest.exports.panic_code.value, 0);

// A valid boundary shift must preserve the signed-i64 bit pattern.
const boundaryShift = compile('pub fnc main(): int { return 1 <<< 63; }');
assert.equal(boundaryShift.ok, true, boundaryShift.diagnostic || 'shift 63 admission');
const boundaryGuest = new WebAssembly.Instance(new WebAssembly.Module(boundaryShift.guest), { atto: {} });
assert.equal(boundaryGuest.exports.main(), -9223372036854775808n);
assert.equal(boundaryGuest.exports.panic_code.value, 0);

// The native and wasm32-hosted compilers must preserve checked integer division
// and keep explicit floating division distinct.
const divisionSource = `fnc half(float numerator, float denominator): float {
  return numerator / denominator;
}
pub fnc main(): int {
  if half(5.0, 2.0) eq 2.5 then
    if -7 / 3 eq -2 then return 42; fi
  fi
  return 1;
}`;
const division = compile(divisionSource);
assert.equal(division.ok, true, division.diagnostic || 'checked quotient compile');
const divisionModule = new WebAssembly.Module(division.guest);
assert.deepEqual(WebAssembly.Module.imports(divisionModule), []);
const divisionGuest = new WebAssembly.Instance(divisionModule, { atto: {} });
assert.equal(divisionGuest.exports.main(), 42n);
assert.equal(divisionGuest.exports.panic_code.value, 0);

for (const [source, result] of [
  ['pub fnc main(): int { return 5 / 2; }', 2n],
  ['pub fnc main(): int { return -5 / 2; }', -2n],
  ['pub fnc main(): int { return 9007199254740993 / 1; }', 9007199254740993n],
  ['pub fnc main(): int { return Integer.rem(-7, 3); }', -1n],
  ['pub fnc main(): int { return Integer.mod(-7, -3); }', 2n],
  ['pub fnc main(): int { return Integer.rem(-9223372036854775808, -1); }', 0n],
  ['pub fnc main(): int { return Integer.mod(-1, -9223372036854775808); }', 9223372036854775807n],
]) {
  const compiled = compile(source);
  assert.equal(compiled.ok, true, compiled.diagnostic);
  const guest = new WebAssembly.Instance(new WebAssembly.Module(compiled.guest), {});
  assert.equal(guest.exports.main(), result);
  assert.equal(guest.exports.panic_code.value, 0);
}
for (const [source, code] of [
  ['fnc q(int d): int { return 7 / d; } pub fnc main(): int { return q(0); }', 2],
  ['fnc q(int d): int { return -9223372036854775808 / d; } pub fnc main(): int { return q(-1); }', 1],
  ['fnc q(int d): int { return Integer.rem(7, d); } pub fnc main(): int { return q(0); }', 2],
  ['fnc q(int d): int { return Integer.mod(7, d); } pub fnc main(): int { return q(0); }', 2],
]) {
  const compiled = compile(source);
  assert.equal(compiled.ok, true, compiled.diagnostic);
  const guest = new WebAssembly.Instance(new WebAssembly.Module(compiled.guest), {});
  assert.throws(() => guest.exports.main(), WebAssembly.RuntimeError);
  assert.equal(guest.exports.panic_code.value, code, 'provisional terminal fault ABI');
}
// Exact shared source, not rewritten for this backend. Failures are still
// provisional terminal fault codes, not ordinary typed Atto Error values.
const sharedNumeric = [
  {
    "id": "checked-i64-quotient",
    "blob": "d7b61afd0fe074395b092aa4abaa08de4961602e",
    "source": "// Candidate guest source: integer truncation and dividend-signed remainder.\n// Target main return 42. Not yet compiler/AOT executed.\npub fnc divide(int lhs, int rhs): int {\n  return lhs / rhs;\n}\npub fnc remainder(int lhs, int rhs): int {\n  return Integer.rem(lhs, rhs);\n}\npub fnc main(): int {\n  val int a = -7;\n  val int b = 3;\n  return divide(a, b) * -21 + (remainder(a, b) + 1);\n}\n",
    "expected": {
      "kind": "value",
      "value": "42"
    }
  },
  {
    "id": "reject-i64-zero-divisor",
    "blob": "6736c0fb356074a753406efe8277aaed724892a3",
    "source": "// Candidate guest source: must fail with division_by_zero (not host exception).\n// Backend may issue a statically proved diagnostic or a typed guest runtime fault.\npub fnc checked(int divisor): int {\n  return 7 / divisor;\n}\npub fnc main(): int {\n  return checked(0);\n}\n",
    "expected": {
      "kind": "error",
      "code": "division_by_zero"
    }
  },
  {
    "id": "reject-i64-min-overflow",
    "blob": "aff2c8170924a3866c8e4f8ec095560148f8b9b5",
    "source": "// Candidate guest source: MIN / -1 must be checked overflow, never MIN.\npub fnc checked(int divisor): int {\n  val int minimum = -9223372036854775808;\n  return minimum / divisor;\n}\npub fnc main(): int {\n  return checked(-1);\n}\n",
    "expected": {
      "kind": "error",
      "code": "integer_overflow"
    }
  },
  {
    "id": "reject-i64-zero-remainder",
    "blob": "a0325c64972b42825f6a82f5c2c941c67dcec7d1",
    "source": "// Candidate guest source: integer remainder by zero is division_by_zero.\npub fnc checked(int divisor): int {\n  return Integer.rem(7, divisor);\n}\npub fnc main(): int {\n  return checked(0);\n}\n",
    "expected": {
      "kind": "error",
      "code": "division_by_zero"
    }
  }
];
for (const c of sharedNumeric) {
  const bytes = Buffer.from(c.source);
  assert.equal(createHash('sha1').update('blob '+bytes.length+'\0').update(bytes).digest('hex'),c.blob);
  const compiled = compile(c.source);
  assert.equal(compiled.ok,true,c.id+': '+compiled.diagnostic);
  const guest = new WebAssembly.Instance(new WebAssembly.Module(compiled.guest),{});
  if(c.expected.kind==='value'){
    assert.equal(guest.exports.main(),BigInt(c.expected.value),c.id);
    assert.equal(guest.exports.panic_code.value,0);
  }else{
    assert.throws(()=>guest.exports.main(),WebAssembly.RuntimeError);
    assert.equal(guest.exports.panic_code.value,c.expected.code==='integer_overflow'?1:2);
  }
}
// Both compiler targets must refuse unproved lossy mixed ordering and
// still execute exact same-carrier comparisons at the i64 precision boundary.
for (const op of ['<', '<=', '>', '>=']) {
  for (const [a, b] of [['9007199254740993', '9007199254740992.0'],
                       ['9007199254740992.0', '9007199254740993']]) {
    const mixed = compile(`pub fnc main(): int { if ${a} ${op} ${b} then return 42; fi return 1; }`);
    assert.equal(mixed.ok, false, 'mixed ordering must not emit a rounded guest');
    assert.match(mixed.diagnostic, /E_TYPE/);
  }
}
for (const body of ['9007199254740993 > 9007199254740992', '2.5 > 2.0']) {
  const ordered = compile(`pub fnc main(): int { if ${body} then return 42; fi return 1; }`);
  assert.equal(ordered.ok, true, ordered.diagnostic);
  const guest = new WebAssembly.Instance(new WebAssembly.Module(ordered.guest), {});
  assert.equal(guest.exports.main(), 42n);
  assert.equal(guest.exports.panic_code.value, 0);
}
const rejectedTruncation = compile('pub fnc main(): int { return 5.0 / 2.0; }');
assert.equal(rejectedTruncation.ok, false, 'explicit float result must not narrow to int');
assert.match(rejectedTruncation.diagnostic, /E_TYPE|E_RETURN_TYPE/);

const rejectedShift = compile('pub fnc main(): int { return 1 <<< 64; }');
assert.equal(rejectedShift.ok, false, 'invalid constant shift must not emit Wasm');
assert.match(rejectedShift.diagnostic, /E_SHIFT_DISTANCE/);
const rejectedNegativeShift = compile('pub fnc main(): int { return 1 >>> -1; }');
assert.equal(rejectedNegativeShift.ok, false, 'negative shift must not emit Wasm');
assert.match(rejectedNegativeShift.diagnostic, /E_SHIFT_DISTANCE/);
const rejectedBooleanCaret = compile('pub fnc main(): int { val b = true ^^ false; return 42; }');
assert.equal(rejectedBooleanCaret.ok, false, 'bool ^^ must not be bitwise integer XOR');
assert.match(rejectedBooleanCaret.diagnostic, /E_TYPE/);


const runtimeShift = compile('pub fnc main(): int { let distance: int = 64; return 1 <<< distance; }');
assert.equal(runtimeShift.ok, true, runtimeShift.diagnostic || 'dynamic checked shift compile');
const checkedShift = new WebAssembly.Instance(new WebAssembly.Module(runtimeShift.guest), { atto: {} });
assert.throws(() => checkedShift.exports.main(), WebAssembly.RuntimeError);
assert.equal(checkedShift.exports.panic_code.value, 7, 'must not mask dynamic shift by 63');
const runtimeNegativeShift = compile('pub fnc main(): int { let distance: int = -1; return 1 >>> distance; }');
assert.equal(runtimeNegativeShift.ok, true, runtimeNegativeShift.diagnostic || 'negative computed shift admission');
const negativeShift = new WebAssembly.Instance(new WebAssembly.Module(runtimeNegativeShift.guest), { atto: {} });
assert.throws(() => negativeShift.exports.main(), WebAssembly.RuntimeError);
assert.equal(negativeShift.exports.panic_code.value, 7, 'negative dynamic count must not mask to 63');

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
  const compoundSource = join(tmp, 'compound-xor.atto');
  const compoundArtifact = join(tmp, 'compound-xor.wasm');
  await writeFile(compoundSource, compoundXorSource);
  const nativeCompound = spawnSync(attoc, [compoundSource, '-o', compoundArtifact],
    { encoding: 'utf8', timeout: 30000 });
  assert.equal(nativeCompound.status, 0, nativeCompound.stderr);
  assert.deepEqual(await readFile(compoundArtifact), Buffer.from(compoundXor.guest),
    'native and wasm32 compilers must emit identical compound XOR guests');
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
console.log('PASS website bitwise Wasm guest, native bytes, checked integer and explicit floating division, checked shifts and source rejection');
