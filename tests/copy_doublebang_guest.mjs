// Compiles real .atto copy!! programs using native attoc and executes guest
// WebAssembly. This is a BOUNDED flat-class test, not a recursive graph proof.
import assert from 'node:assert/strict';
import { readFile, mkdtemp, rm, stat } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const attoc = process.env.ATTOC || fileURLToPath(new URL('../target/debug/attoc', import.meta.url));
const positives = [
  '991-copy-doublebang-scalar.atto',
  '992-copy-doublebang-grouped.atto',
  '993-copy-doublebang-flat-class.atto',
  '994-copy-doublebang-two-independent-class-copies.atto',
  '995-copy-doublebang-flat-class-mixed-fields.atto',
  '996-copy-doublebang-constructor-state.atto',
  '997-copy-doublebang-private-state.atto',
];
const negatives = [
  '991-copy-doublebang-class-unproved.atto',
  '992-copy-doublebang-mut-invalid.atto',
  '996-copy-doublebang-exclusive-loan.atto',
];
const imports = {
  atto: {
    print_i64: () => { throw new Error('unexpected guest print_i64'); },
    print_bool: () => { throw new Error('unexpected guest print_bool'); },
    print_f64: () => { throw new Error('unexpected guest print_f64'); },
    print_str: () => { throw new Error('unexpected guest print_str'); },
    print_newline: () => { throw new Error('unexpected guest print_newline'); },
    actor_spawn: () => { throw new Error('unexpected guest actor_spawn'); },
    actor_send: () => { throw new Error('unexpected guest actor_send'); },
  },
};

function run(source, output) {
  return spawnSync(attoc, [source, '-o', output], {
    encoding: 'utf8', timeout: 30_000, maxBuffer: 1 << 20,
  });
}

const dir = await mkdtemp(join(tmpdir(), 'atto-copy-doublebang-'));
try {
  for (const filename of positives) {
    const source = fileURLToPath(new URL('./fixtures/pass/' + filename, import.meta.url));
    const text = await readFile(source, 'utf8');
    const expected = text.match(/^\/\/ expect-exit: (\d+)$/m)?.[1];
    assert.ok(expected, filename + ': missing return-value expectation');
    const output = join(dir, filename + '.wasm');
    const result = run(source, output);
    assert.equal(result.error, undefined, filename + ': compiler process failed');
    assert.equal(result.status, 0, filename + ': compiler rejected: ' + result.stderr);
    const wasm = await readFile(output);
    assert.ok(WebAssembly.validate(wasm), filename + ': invalid guest wasm');
    const module = new WebAssembly.Module(wasm);
    for (const binding of WebAssembly.Module.imports(module)) {
      assert.equal(binding.module, 'atto', filename + ': external import');
      assert.ok(Object.hasOwn(imports.atto, binding.name), filename + ': unapproved guest import ' + binding.name);
    }
    const guest = new WebAssembly.Instance(module, imports);
    assert.equal(guest.exports.main(), BigInt(expected), filename + ': wrong guest result');
    assert.equal(guest.exports.panic_code.value, 0, filename + ': guest panic_code');
    console.log('PASS compiled guest', filename, expected);
  }
  for (const filename of negatives) {
    const source = fileURLToPath(new URL('./fixtures/fail/' + filename, import.meta.url));
    const text = await readFile(source, 'utf8');
    const expected = text.match(/^\/\/ expect-error: (E_[A-Z_]+)$/m)?.[1];
    assert.ok(expected, filename + ': missing error-code expectation');
    const output = join(dir, filename + '.wasm');
    const result = run(source, output);
    assert.equal(result.error, undefined, filename + ': compiler process failed');
    assert.notEqual(result.status, 0, filename + ': unsupported copy accepted');
    assert.ok(result.stderr.includes(expected), filename + ': expected ' + expected + ', got: ' + result.stderr);
    await assert.rejects(stat(output), { code: 'ENOENT' }, filename + ': rejected compiler produced output');
    console.log('PASS rejected guest', filename, expected);
  }
  console.log('COPY_DEEP_GUEST_RESULT pass=' + positives.length + ' reject=' + negatives.length);
} finally {
  await rm(dir, { recursive: true, force: true });
}
