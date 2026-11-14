// Exercise the actual wasm32 compiler and resulting guest in Node.
// Do not interpret guest source in JavaScript or pretend generator runtime exists.
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';

const path = process.argv[2] ?? 'target/wasm32-unknown-unknown/release/atto_compiler.wasm';
const compilerBytes = await readFile(path);
const module = new WebAssembly.Module(compilerBytes);
assert.deepEqual(WebAssembly.Module.imports(module), [], 'compiler must have zero imports');
const compiler = new WebAssembly.Instance(module, {}).exports;
assert.equal(compiler.abi_version(), 2);
const encoder = new TextEncoder(), decoder = new TextDecoder();

function compile(source) {
  const input = encoder.encode(source);
  assert.ok(input.length < compiler.input_capacity());
  new Uint8Array(compiler.memory.buffer, compiler.input_ptr(), input.length).set(input);
  const code = compiler.compile(input.length);
  const outputSize = compiler.result_length();
  if (code !== 0) {
    assert.equal(outputSize, 0, 'failed compilation must never publish a guest');
    const ptr = compiler.error_ptr();
    const messageBytes = new Uint8Array(compiler.memory.buffer, ptr, 512);
    const end = messageBytes.indexOf(0);
    return {ok: false, message: decoder.decode(messageBytes.subarray(0, end < 0 ? 512 : end))};
  }
  const guestBytes = new Uint8Array(compiler.memory.buffer, compiler.result_ptr(), outputSize).slice();
  assert.ok(WebAssembly.validate(guestBytes));
  return {ok: true, guestBytes};
}

const addition = compile('pub fnc main(): int { let sum = 0; sum += 41; return sum + 1; }');
assert.equal(addition.ok, true, addition.message);
const guestModule = new WebAssembly.Module(addition.guestBytes);
assert.deepEqual(WebAssembly.Module.imports(guestModule), []);
assert.equal(new WebAssembly.Instance(guestModule, {}).exports.main(), 42n);
console.log('PASS: wasm32 compiler emitted guest with checked compound addition returning 42');

const generators = [
  'fnc *gen count_to(int limit): Generator<int> {',
  '  for (let i = 1; i <= limit; i++) do yield i; done',
  '}',
  'fnc *gen count_with_zero(int limit): Generator<int> {',
  '  yield 0;',
  '  yield* count_to(limit);',
  '}',
  'pub fnc main(): int {',
  '  let sum = 0;',
  '  for value in count_with_zero(4) do sum += value; done',
  '  stdio.println(sum);',
  '  return sum;',
  '}',
].join('\n');
// Generator delegation is now an admitted, executable bounded profile.
const delegated = compile(generators);
assert.equal(delegated.ok, true, delegated.message);
assert.deepEqual(compile(generators).guestBytes, delegated.guestBytes,
  'generator output must be deterministic');
const delegatedModule = new WebAssembly.Module(delegated.guestBytes);
const imports = WebAssembly.Module.imports(delegatedModule);
assert.ok(imports.length > 0, 'stdio output requires a declared import');
assert.ok(imports.every(x => x.module === 'atto'
  && ['print_i64', 'print_newline'].includes(x.name)
  && x.kind === 'function'), 'guest uses only approved print imports');
let stdout = '';
const delegatedGuest = new WebAssembly.Instance(delegatedModule, { atto: {
  print_i64: n => { stdout += String(n); },
  print_newline: () => { stdout += '\n'; },
}});
assert.equal(delegatedGuest.exports.main(), 10n,
  'yield*, iteration and checked summation execute in guest Wasm');
assert.equal(delegatedGuest.exports.panic_code.value, 0);
assert.equal(stdout, '10\n');
console.log('PASS: wasm32 compiler emits an executing generator delegation guest returning 10');

// Unsupported generator profiles still fail closed and publish no guest artifact.
const unloweredSource = await readFile(
  new URL('./fixtures/fail/128-generator-delegation-not-yet-lowered.atto', import.meta.url),
  'utf8');
const unlowered = compile(unloweredSource);
assert.equal(unlowered.ok, false, 'unimplemented generator profiles must reject');
assert.match(unlowered.message, /^E_UNSUPPORTED:/);
console.log('PASS: unsupported generator profile still fails closed');
