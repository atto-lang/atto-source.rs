// Exact site language previews, executed against the real wasm32 compiler.
// Compiler admission != semantic parity; do not count parser-only success as run.
import assert from 'node:assert/strict';
import { readFile, readdir } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';

const corpus = new URL('./playground_previews/', import.meta.url);
const imports = new Set([
  'print_i64', 'print_bool', 'print_str', 'print_newline', 'print_f64',
  'actor_spawn', 'actor_send'
]);
function gitBlobSha(bytes) {
  return createHash('sha1').update(Buffer.from('blob ' + bytes.length + '\0'))
    .update(bytes).digest('hex');
}

export async function auditPlaygroundPreviews({ compile, runGuest, nativeBytes = null }) {
  const raw = await readFile(new URL('MANIFEST.tsv', corpus), 'utf8');
  const pairs = raw.split('\n').filter(line => line && !line.startsWith('#'))
    .map(line => line.split('\t'));
  assert.equal(pairs.length, 35, 'every public preview has a pinned blob');
  assert.ok(pairs.every(([name, sha]) =>
    /^[a-z-]+$/.test(name) && /^[a-f0-9]{40}$/.test(sha)));
  const names = pairs.map(([name]) => name);
  assert.equal(new Set(names).size, 35, 'no duplicate preview names');
  assert.deepEqual((await readdir(corpus)).filter(n => n.endsWith('.atto')).sort(),
    names.map(name => name + '.atto').sort());
  const outcome = { total: 35, admitted: [], unsupported: [],
    guestExecuted: [], nativeCompared: 0, parityCertified: false };
  for (const [name, expectedSha] of pairs) {
    const url = new URL(name + '.atto', corpus);
    const sourceBytes = await readFile(url);
    assert.equal(gitBlobSha(sourceBytes), expectedSha,
      name + ': website byte-for-byte Git blob parity');
    const source = sourceBytes.toString('utf8');
    const result = compile(source);
    if (!result.ok) {
      assert.equal('wasm' in result, false, name + ': no artifact on error');
      assert.match(result.code, /^E_[A-Z_]+$/, name + ': diagnostic code');
      assert.ok(result.line >= 1 && result.col >= 1, name + ': source position');
      outcome.unsupported.push({ name, code: result.code });
      if (nativeBytes) {
        assert.equal(nativeBytes(fileURLToPath(url)), null,
          name + ': native host must reject same source');
        outcome.nativeCompared++;
      }
      continue;
    }
    outcome.admitted.push(name);
    assert.ok(WebAssembly.validate(result.wasm), name + ': real valid guest');
    assert.deepEqual(compile(source).wasm, result.wasm,
      name + ': deterministic compile');
    const mod = new WebAssembly.Module(result.wasm);
    for (const imp of WebAssembly.Module.imports(mod)) {
      assert.equal(imp.module, 'atto', name + ': unexpected host module');
      assert.ok(imports.has(imp.name), name + ': unexpected host import ' + imp.name);
    }
    if (nativeBytes) {
      assert.deepEqual(nativeBytes(fileURLToPath(url)), result.wasm,
        name + ': native/wasm32 byte parity');
      outcome.nativeCompared++;
    }
    // Only source examples with a fixed, independently reviewed output oracle
    // count as executed. Admission without execution remains unverified.
    const expected = {
      'function-pipeline': { value: 121n, stdout: '121\n' },
      'generators': { value: 10n, stdout: '10\n' },
      'result-option': { value: 0n, stdout: '8080\n' },
      'trap': { value: 7n, stdout: '5\nid must be nonnegative\n' },
    }[name];
    if (expected) {
      const guest = runGuest(result.wasm);
      assert.ok(guest.ok, name + ': guest trap ' + guest.error);
      assert.equal(guest.value, expected.value, name + ': exit value');
      assert.equal(guest.stdout, expected.stdout, name + ': stdout');
      outcome.guestExecuted.push(name);
    }
  }
  assert.equal(outcome.admitted.length + outcome.unsupported.length, 35);
  assert.deepEqual(outcome.guestExecuted.slice().sort(),
    ['function-pipeline', 'generators', 'result-option', 'trap'].sort(),
    'all four currently accepted public previews must execute');
  return outcome;
}
