// Runs native-compiler-generated Wasm guests for all currently implemented callable forms.
// In particular, this checks *execution* rather than just compiler acceptance.
import assert from 'node:assert/strict';
import { readFile, mkdtemp, rm } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const attoc = fileURLToPath(new URL('../target/debug/attoc', import.meta.url));
const fixtures = [
  '165-callable-five-function-forms.atto',
  '166-callable-five-routine-forms.atto',
  '167-callable-five-inline-method-forms.atto',
  '168-callable-end-nested-arg-commas.atto',
];
const negative = [
  '39-end-form-body.atto',
  '165-callable-trailing-comma.atto',
  '166-callable-colon-colon-brace.atto',
  '167-callable-brace-comma.atto',
];
const allowed = new Set(['print_i64','print_bool','print_str','print_newline','print_f64','actor_spawn','actor_send']);
const imports = {
  atto: {
    print_i64: () => { throw Error('unexpected printing'); },
    print_bool: () => { throw Error('unexpected printing'); },
    print_str: () => { throw Error('unexpected printing'); },
    print_newline: () => { throw Error('unexpected printing'); },
    print_f64: () => { throw Error('unexpected printing'); },
    actor_spawn: () => { throw Error('unexpected actor spawn'); },
    actor_send: () => { throw Error('unexpected actor send'); },
  }
};
const run = (source, out) => spawnSync(attoc, [source, '-o', out], { encoding: 'utf8', timeout: 30_000, maxBuffer: 1 << 20 });
const work = await mkdtemp(join(tmpdir(), 'atto-callable-'));
try {
  for (const name of fixtures) {
    const path = fileURLToPath(new URL('./fixtures/pass/' + name, import.meta.url));
    const source = await readFile(path, 'utf8');
    const want = source.match(/^\/\/ expect-exit: (\d+)/m)?.[1];
    assert.ok(want, name + ': expected exit header missing');
    const out = join(work, name + '.wasm');
    const built = run(path, out);
    assert.equal(built.status, 0, name + ': attoc failed: ' + built.stderr);
    const wasm = await readFile(out);
    assert.ok(WebAssembly.validate(wasm), name + ': invalid guest');
    const module = new WebAssembly.Module(wasm);
    for (const imp of WebAssembly.Module.imports(module)) {
      assert.equal(imp.module, 'atto', name + ': wrong import module');
      assert.ok(allowed.has(imp.name), name + ': unexpected import ' + imp.name);
    }
    const guest = new WebAssembly.Instance(module, imports);
    const actual = guest.exports.main();
    assert.equal(actual, BigInt(want), name + ': wrong guest return value');
    assert.equal(guest.exports.panic_code.value, 0, name + ': unexpected panic');
    console.log('PASS guest execution', name, String(actual));
  }
  for (const name of negative) {
    const path = fileURLToPath(new URL('./fixtures/fail/' + name, import.meta.url));
    const source = await readFile(path, 'utf8');
    const want = source.match(/^\/\/ expect-error: (E_\w+)/m)?.[1];
    assert.ok(want, name + ': expected error header missing');
    const output = join(work, name + '.wasm');
    const built = run(path, output);
    assert.equal(built.status, 1, name + ': expected compiler rejection');
    assert.ok(built.stderr.includes(want), name + ': missing ' + want + ': ' + built.stderr);
    console.log('PASS expected diagnostic', name, want);
  }
} finally {
  await rm(work, { recursive: true, force: true });
}
