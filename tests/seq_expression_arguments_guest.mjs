// Direct guest-Wasm execution and native-vs-wasm32 compiler byte parity for
// comma-only Atto seq expressions. This invokes the REAL compiler twice;
// it never interprets AttoLang in JavaScript.
import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {mkdtemp, readFile, writeFile, rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';

const attoc = resolve(process.env.ATTOC || 'target/release/attoc');
const compilerPath = resolve(process.env.ATTO_COMPILER_WASM ||
  'target/wasm32-unknown-unknown/release/atto_compiler.wasm');
const compilerBytes = await readFile(compilerPath);
assert.ok(WebAssembly.validate(compilerBytes), 'wasm32 compiler must be valid');
const compilerModule = new WebAssembly.Module(compilerBytes);
assert.equal(WebAssembly.Module.imports(compilerModule).length, 0,
  'compiler must be pure and import no host capabilities');
const compiler = new WebAssembly.Instance(compilerModule, {}).exports;
assert.equal(compiler.abi_version(), 2, 'unexpected compiler ABI');

function compileWasmHost(source) {
  const bytes = new TextEncoder().encode(source);
  assert.ok(bytes.length < compiler.input_capacity(), 'fixture exceeds input budget');
  new Uint8Array(compiler.memory.buffer, compiler.input_ptr(), bytes.length).set(bytes);
  const status = compiler.compile(bytes.length);
  if (status !== 0) {
    assert.equal(compiler.result_length(), 0, 'rejected program emitted guest bytes');
    throw new Error('wasm32 compiler rejected positive seq fixture');
  }
  const guest = Buffer.from(new Uint8Array(
    compiler.memory.buffer, compiler.result_ptr(), compiler.result_length()).slice());
  assert.ok(WebAssembly.validate(guest), 'wasm32 compiler emitted invalid guest');
  return guest;
}

function executeGuest(bytes) {
  assert.ok(WebAssembly.validate(bytes), 'native compiler emitted invalid guest');
  const mod = new WebAssembly.Module(bytes);
  const events = [];
  const imports = {atto: {}};
  const declared = new Set(['print_i64', 'print_bool', 'print_str', 'print_newline',
    'print_f64', 'actor_spawn', 'actor_send', 'state_register', 'state_enter', 'state_leave']);
  for (const imp of WebAssembly.Module.imports(mod)) {
    assert.equal(imp.module, 'atto', 'unexpected import namespace');
    assert.ok(imp.kind === 'function' && declared.has(imp.name),
      'unexpected guest capability ' + imp.name);
    if (imp.name === 'print_i64') imports.atto[imp.name] = n => events.push(String(n));
    else if (imp.name === 'print_newline') imports.atto[imp.name] = () => events.push('newline');
    else imports.atto[imp.name] = () => {throw Error('unexpected guest import ' + imp.name)};
  }
  const instance = new WebAssembly.Instance(mod, imports);
  assert.equal(typeof instance.exports.main, 'function');
  const result = instance.exports.main();
  assert.equal(instance.exports.panic_code?.value, 0, 'guest panicked');
  assert.equal(typeof result, 'bigint', 'main must return a Wasm i64');
  return {result, events};
}

const fixtures = [
  ['delimited', 'pub fnc main(): int { return seq(1, 42); }', 42],
  ['bare_call', 'fnc calculate(int v): int { return v; } pub fnc main(): int { return calculate(seq 1, 2, 42); }', 42],
  ['multiline_call', 'fnc calculate(int v): int { return v; } pub fnc main(): int { return calculate(\n seq\n 1,\n 2,\n 42\n); }', 42],
  ['multi_argument', 'fnc add(int a, int b): int { return a + b; } pub fnc main(): int { return add(seq(1, 2), 40); }', 42],
  ['bare_final_argument', 'fnc add(int a, int b): int { return a + b; } pub fnc main(): int { return add(40, seq 1, 2); }', 42],
  ['mutable_bare_argument', 'fnc foo(int v): int { return v * 2; } pub fnc main(): int { let int z = 10; const int x = 5; return foo(seq z += x, z); }', 30],
  ['mutable_delimited_argument', 'fnc foo(int v): int { return v * 2; } pub fnc main(): int { let int z = 10; const int x = 5; return foo(seq(z += x, z)); }', 30],
  ['ordered_assignment', 'pub fnc main(): int { let int z = 0; const int a = seq(z += 1, z); const int b = seq(z += 1, z); return z * 10 + a + b; }', 23],
  ['shorthand_mutation', 'pub fnc main(): int { let int z = 3; return (> z += 4, z * 6); }', 42],
  ['nested_seq_scope', 'pub fnc main(): int { const int n = seq(let int x = 10, seq(let int y = 2, x * y)); return n + 22; }', 42],
  ['multiple_mutations', 'pub fnc main(): int { let int z = 0; return seq(z += 1, z += 2, z * 14); }', 42],
  ['argument_order', 'fnc sub(int a,int b): int { return b-a; } pub fnc main(): int { let int z = 1; return sub(seq(z += 1, z), z * 10); }', 18],
  ['nested_bare_call', 'fnc add(int a,int b): int { return a+b; } fnc id(int v): int { return v; } pub fnc main(): int { return id(seq 1, add(2, 40)); }', 42],
  ['mutation_stdout_order', 'pub fnc main(): int { let int z = 0; return seq(stdio.println(11), z += 20, stdio.println(z), z + 22); }', 42, ['11', 'newline', '20', 'newline']],
  ['once_only_print', 'pub fnc main(): int { return seq(stdio.println(10), stdio.println(20), 42); }', 42, ['10', 'newline', '20', 'newline']],
];
const work = await mkdtemp(join(tmpdir(), 'atto-seq-guest-'));
let passed = 0;
try {
  for (const [id, source, expected, expectedEvents] of fixtures) {
    const input = join(work, id + '.atto'), output = join(work, id + '.wasm');
    await writeFile(input, source, 'utf8');
    const native = spawnSync(attoc, [input, '-o', output],
      {encoding: 'utf8', timeout: 30000, maxBuffer: 1 << 20});
    assert.equal(native.status, 0, `native compiler failed ${id}: ${native.stderr || native.error}`);
    const nativeBytes = await readFile(output);
    const wasmBytes = compileWasmHost(source);
    assert.deepEqual(nativeBytes, wasmBytes, `native/wasm32 guest bytes diverged: ${id}`);
    const {result, events} = executeGuest(nativeBytes);
    assert.equal(result, BigInt(expected), `incorrect executed guest result: ${id}`);
    if (expectedEvents) assert.deepEqual(events, expectedEvents, `effect order: ${id}`);
    else assert.deepEqual(events, [], `unexpected host effects: ${id}`);
    passed++;
    console.log('SEQ_GUEST_PASS', id, 'value=' + String(result));
  }
  assert.equal(passed, fixtures.length, 'incomplete guest conformance');
  console.log('SEQ_GUEST_RECEIPT', JSON.stringify({passed, failed: 0,
    native_wasm_byte_parity: true, guest_executed: true}));
} finally {
  await rm(work, {recursive:true, force:true});
}
