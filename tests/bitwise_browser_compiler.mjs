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
  assert.equal(result.error, undefined, 'native compiler infrastructure failure');
  assert.equal(result.signal, null, 'native compiler terminated by signal');
  assert.equal(result.status, 0, result.stderr);
  const nativeBytes = await readFile(out);
  assert.deepEqual(Buffer.from(compiled.guest), nativeBytes,
    'native and wasm32 compilers must emit byte-identical guest for exact preview');

  // Carry forward #411's unique signed compound boundaries without
  // replacing this branch's named Boolean reduction implementation.
  const vectors = [
    ['ordinary', 'let x: int = 12; x ^^= 10; return x;', 6n],
    ['min-max', 'let x: int = -9223372036854775808; x ^^= -1; return x;', 9223372036854775807n],
    ['max-min', 'let x: int = 9223372036854775807; x ^^= -1; return x;', -9223372036854775808n],
    ['self', 'let x: int = -9223372036854775808; x ^^= x; return x;', 0n],
    ['wide', 'let x: int = 9007199254740993; x ^^= 9007199254740992; return x;', 1n],
  ];
  for (const [id, body, expected] of vectors) {
    const sourceText = `pub fnc main(): int { ${body} }\n`;
    const browser = compile(sourceText);
    assert.equal(browser.ok, true, `${id}: ${browser.diagnostic || ''}`);
    const guestModule = new WebAssembly.Module(browser.guest);
    assert.deepEqual(WebAssembly.Module.imports(guestModule), [], id);
    const guest = new WebAssembly.Instance(guestModule, {});
    assert.equal(guest.exports.main(), expected, id);
    assert.equal(guest.exports.panic_code.value, 0, id);
    const src = join(tmp, `${id}.atto`);
    const bin = join(tmp, `${id}.wasm`);
    await writeFile(src, sourceText);
    const native = spawnSync(attoc, [src, '-o', bin], { encoding: 'utf8', timeout: 30000 });
    assert.equal(native.error, undefined, id);
    assert.equal(native.signal, null, id);
    assert.equal(native.status, 0, `${id}: ${native.stderr}`);
    assert.deepEqual(Buffer.from(browser.guest), await readFile(bin), id);
  }
  for (const [index, body] of [
    'val x: int = 12; x ^^= 10; return x;',
    'let x: int = 12; x ^^= true; return x;',
    'let x: int = 12; x ^^= 1.5; return x;',
    'let x: float = 12.0; x ^^= 10; return 0;',
    'let x: int = 12; x ^= 10; return x;',
    'let x: int = 12; x ^^ = 10; return x;',
    'let x: int = 12; x ^/*gap*/^= 10; return x;',
  ].entries()) {
    const sourceText = `pub fnc main(): int { ${body} }\n`;
    const browser = compile(sourceText);
    assert.equal(browser.ok, false, body);
    assert.ok(browser.diagnostic.trim(), body);
    const src = join(tmp, `reject-${index}.atto`);
    const bin = join(tmp, `reject-${index}.wasm`);
    await writeFile(src, sourceText);
    const native = spawnSync(attoc, [src, '-o', bin], { encoding: 'utf8', timeout: 30000 });
    assert.equal(native.error, undefined, body);
    assert.equal(native.signal, null, body);
    assert.equal(native.status, 1, body);
    assert.ok(native.stderr.trim(), body);
    await assert.rejects(readFile(bin), { code: 'ENOENT' }, body);
    // A failed browser compilation must not poison the next invocation.
    const recovered = compile('pub fnc main(): int { return 42; }');
    assert.equal(recovered.ok, true, body);
    const guest = new WebAssembly.Instance(new WebAssembly.Module(recovered.guest), {});
    assert.equal(guest.exports.main(), 42n, body);
    assert.equal(guest.exports.panic_code.value, 0, body);
  }
  console.log('PASS 5 compound guest byte-parity cases; 7 dual-compiler source rejections and browser recoveries');
} finally {
  await rm(tmp, { recursive: true, force: true });
}

for (const [src, expected] of [
  ['pub fnc main(): int { return 0b2; }', 'E_INTEGER_LITERAL'],
  ['pub fnc main(): int { return 1 ^^ true; }', 'E_TYPE'],
  ['pub fnc main(): int { return 0b; }', 'E_INTEGER_LITERAL'],
]) {
  const failure = compile(src);
  assert.equal(failure.ok, false, 'negative source cannot return a Wasm artifact');
  assert.ok(failure.diagnostic.includes(expected), failure.diagnostic);
}
console.log('PASS exact website bitwise source: wasm32 compiler -> guest Wasm = 188; native bytes equal; 3 negatives');
