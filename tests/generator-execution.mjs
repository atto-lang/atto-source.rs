// Actual native-compiled and wasm32-host-compiled guest Wasm execution proof.
// The tests intentionally import only allowed print_* host functions; all
// generator allocate/next/close work stays inside the guest Wasm module.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { join } from 'node:path';

const attoc = process.env.ATTOC || 'target/release/attoc';
const compilerWasm = process.env.ATTO_COMPILER_WASM ||
  'target/wasm32-unknown-unknown/release/atto_compiler.wasm';
const api = new WebAssembly.Instance(new WebAssembly.Module(readFileSync(compilerWasm)), {}).exports;
assert.equal(api.abi_version(), 2);
const enc = new TextEncoder();
const dec = new TextDecoder('utf8', { fatal: true });

function viaWasmCompiler(source) {
  const input = enc.encode(source);
  new Uint8Array(api.memory.buffer, api.input_ptr(), input.length).set(input);
  const code = api.compile(input.length);
  if (code !== 0) return { rejected: true, code: (() => {
    const data = new Uint8Array(api.memory.buffer);
    let i = api.error_ptr(); while (data[i]) i++;
    return dec.decode(data.subarray(api.error_ptr(), i)).split(':')[0];
  })() };
  return { rejected: false, wasm: new Uint8Array(
    api.memory.buffer, api.result_ptr(), api.result_length()).slice() };
}

function viaNativeCompiler(file) {
  const result = spawnSync(attoc, [file], { maxBuffer: 1 << 22 });
  if (result.status !== 0) {
    return { rejected: true, code: String(result.stderr).match(/\bE_[A-Z_]+\b/)?.[0] };
  }
  return { rejected: false, wasm: new Uint8Array(result.stdout) };
}

function runGuest(bytes) {
  // Construct before the generic validate assertion: engines provide an
  // exact byte offset and operand-stack reason on invalid generated code.
  const mod = new WebAssembly.Module(bytes);
  assert.ok(WebAssembly.validate(bytes), 'actual guest module must validate');
  let stdout = '';
  const atto = {
    print_i64: n => { stdout += String(n); },
    print_newline: () => { stdout += '\n'; },
    print_bool: v => { stdout += v ? 'true' : 'false'; },
    print_str: () => { throw Error('unexpected string print'); },
    print_f64: () => { throw Error('unexpected float print'); },
    actor_spawn: () => { throw Error('unexpected actor spawn'); },
    actor_send: () => { throw Error('unexpected actor send'); },
  };
  for (const imp of WebAssembly.Module.imports(mod)) {
    assert.equal(imp.module, 'atto');
    assert.ok(Object.hasOwn(atto, imp.name), 'import must be part of the guest ABI');
  }
  const instance = new WebAssembly.Instance(mod, { atto });
  const value = instance.exports.main();
  assert.equal(instance.exports.panic_code.value, 0, 'no guest trap');
  return { value, stdout };
}

for (const [name, expected, output] of [
  ['128-generator-native-yield.atto', 42n, '42\n'],
  ['129-generator-close-reuse.atto', 42n, ''],
  ['130-generator-early-return-close.atto', 42n, ''],
  ['131-generator-empty.atto', 42n, ''],
  ['132-generator-yield-star-native.atto', 10n, '10\n'],
  ['133-generator-nested-empty-yield-star.atto', 8n, ''],
  ['134-generator-delegate-break-close-reuse.atto', 168n, ''],
]) {
  const file = join('tests/fixtures/pass', name);
  const src = readFileSync(file, 'utf8');
  const native = viaNativeCompiler(file);
  const wasmHost = viaWasmCompiler(src);
  assert.equal(native.rejected, false, name + ': native compiler emitted an error');
  assert.equal(wasmHost.rejected, false, name + ': wasm-host compiler emitted an error');
  assert.deepEqual(native.wasm, wasmHost.wasm, name + ': native and wasm-host must be byte-identical');
  assert.deepEqual(runGuest(native.wasm), { value: expected, stdout: output });
  assert.deepEqual(viaWasmCompiler(src).wasm, native.wasm, 'deterministic guest bytes');
  console.log('PASS', name, 'result', String(expected), 'byte parity', native.wasm.length);
}
for (const [name, code] of [
  ['128-generator-delegation-not-yet-lowered.atto', 'E_UNSUPPORTED'],
  ['129-generator-ownership-no-discard.atto', 'E_GENERATOR_OWNERSHIP'],
  ['130-generator-scalar-iteration.atto', 'E_TYPE'],
  ['131-generator-forin-throw-not-safe.atto', 'E_UNSUPPORTED'],
  ['132-generator-delegation-cycle.atto', 'E_GENERATOR_CYCLE'],
  ['133-generator-yield-star-scalar-call.atto', 'E_TYPE'],
]) {
  const file = join('tests/fixtures/fail', name);
  const src = readFileSync(file, 'utf8');
  const native = viaNativeCompiler(file);
  const wasmHost = viaWasmCompiler(src);
  assert.equal(native.rejected, true, name);
  assert.equal(wasmHost.rejected, true, name);
  assert.equal(native.code, code, name);
  assert.equal(wasmHost.code, code, name);
  console.log('PASS expected reject', name, code);
}
