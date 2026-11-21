// Exact-head native/Wasm32 compiler and executable guest proof for the
// closed-world, single-implementor owned Dyn<I> browser profile.
import assert from 'node:assert/strict';
import { mkdtemp, writeFile, readFile, stat, rm } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const attoc = process.env.ATTOC || fileURLToPath(new URL('../target/release/attoc', import.meta.url));
const compilerPath = process.env.ATTO_WASM_COMPILER || fileURLToPath(
  new URL('../target/wasm32-unknown-unknown/release/atto_compiler.wasm', import.meta.url));
const encoder = new TextEncoder();
const decoder = new TextDecoder('utf-8', { fatal: true });

const source = String.raw`
define pub interface Printable as
  describe(): string;
end
define class Greeting implements Printable as
  pub describe(): string { return "hello"; }
  pub secret(): int { return 42; }
end
fnc display(Dyn<Printable> item): string {
  return item.describe();
}
pub fnc main(): int {
  val Greeting concrete = new Greeting();
  val Dyn<Printable> erased = Dyn<Printable>(take! concrete);
  stdio.println(display(erased));
  return 0;
}
`;
const negatives = [
  ['constructor forgery', source.replace('Dyn<Printable>(take! concrete)', 'Dyn<Printable>(42)'), 'E_DYN_OWNERSHIP'],
  ['hidden method', source.replace('stdio.println(display(erased));', 'stdio.println(erased.secret());'), 'E_DYN_INTERFACE'],
  ['after move', source.replace('stdio.println(display(erased));', 'stdio.println(concrete.describe());'), 'E_MOVED'],
  ['implicit widening', source.replace('stdio.println(display(erased));', 'stdio.println(display(concrete));'), 'E_DYN_INTERFACE'],
  ['wrong interface', source.replaceAll('Dyn<Printable>', 'Dyn<Missing>'), 'E_DYN_INTERFACE'],
  ['borrowed erasure', source.replace('Dyn<Printable>(take! concrete)', 'Dyn<Printable>(borrow! concrete)'), 'E_DYN_OWNERSHIP'],
  ['object result', source.replace('describe(): string;', 'describe(): Greeting;'), 'E_DYN_ABI'],
  ['concrete escape', source.replace('stdio.println(display(erased));', 'val Greeting alias = erased;'), 'E_DYN_INTERFACE'],
  ['missing vtable', source.replace('fnc display(', [
    'define class Farewell implements Printable as',
    '  pub describe(): string { return "goodbye"; }',
    'end',
    'fnc display('
  ].join('\n')), 'E_DYN_RUNTIME']
];

const work = await mkdtemp(join(tmpdir(), 'atto-owned-dyn-'));
try {
  const compilerBytes = await readFile(compilerPath);
  assert.ok(WebAssembly.validate(compilerBytes), 'real wasm32 compiler must load');
  const compilerModule = new WebAssembly.Module(compilerBytes);
  assert.deepEqual(WebAssembly.Module.imports(compilerModule), [], 'compiler has no host imports');
  const compiler = new WebAssembly.Instance(compilerModule, {}).exports;
  assert.equal(compiler.abi_version(), 2);
  const compileBrowser = text => {
    const utf8 = encoder.encode(text);
    assert.ok(utf8.length <= compiler.input_capacity(), 'compiler source quota');
    new Uint8Array(compiler.memory.buffer, compiler.input_ptr(), utf8.length).set(utf8);
    const rc = compiler.compile(utf8.length);
    if (rc !== 0) {
      assert.equal(compiler.result_length(), 0, 'compiler failure published guest bytes');
      const memory = new Uint8Array(compiler.memory.buffer);
      const start = compiler.error_ptr();
      let end = start;
      while (end < memory.length && memory[end] !== 0) end++;
      return {ok:false, diagnostic:decoder.decode(memory.subarray(start, end))};
    }
    const guest = new Uint8Array(
      compiler.memory.buffer, compiler.result_ptr(), compiler.result_length()).slice();
    assert.ok(WebAssembly.validate(guest), 'compiled Wasm guest must validate');
    return {ok:true, guest};
  };
  const compileNative = async (text, name) => {
    const path = join(work, name + '.atto');
    const output = join(work, name + '.wasm');
    await writeFile(path, text);
    const proc = spawnSync(attoc, [path, '-o', output], {encoding:'utf8',timeout:30000});
    return {proc, output};
  };
  const {proc, output} = await compileNative(source, 'positive');
  assert.equal(proc.status, 0, 'native compiler rejected Dyn fixture: ' + proc.stderr);
  const nativeGuest = await readFile(output);
  const browserPositive = compileBrowser(source);
  assert.equal(browserPositive.ok, true, 'wasm32 compiler rejected same source');
  assert.deepEqual(Buffer.from(browserPositive.guest), nativeGuest,
    'native and wasm32 compilers must emit the same guest bytes');

  let stdout = '';
  let instance;
  const imported = {atto: {
    print_str: (ptr, len) => {
      stdout += decoder.decode(new Uint8Array(instance.exports.memory.buffer, ptr, len));
    },
    print_newline: () => {stdout += '\n';}
  }};
  ({instance} = await WebAssembly.instantiate(nativeGuest, imported));
  assert.equal(instance.exports.main(), 0n);
  assert.equal(instance.exports.panic_code.value, 0);
  assert.equal(stdout, 'hello\n', 'compiled interface call executes on guest');

  for (const [i, [label, text, diagnostic]] of negatives.entries()) {
    const {proc: failed, output: forbidden} = await compileNative(text, 'negative-' + i);
    assert.notEqual(failed.status, 0, label + ': native compiler must reject');
    assert.ok(failed.stderr.includes(diagnostic), label + ': ' + failed.stderr);
    await assert.rejects(stat(forbidden), {code:'ENOENT'}, label + ': native artifact published');
    const browser = compileBrowser(text);
    assert.equal(browser.ok, false, label + ': browser accepted invalid source');
    assert.ok(browser.diagnostic.includes(diagnostic), label + ': ' + browser.diagnostic);
  }
  console.log('PASS owned Dyn: native/Wasm compiler byte parity, executed hello guest,',
    negatives.length, 'negative no-artifact cases');
} finally {
  await rm(work, {recursive:true,force:true});
}
