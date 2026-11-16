// Exact site language previews, executed against the real wasm32 compiler.
// Compiler admission != semantic parity; do not count parser-only success as run.
import assert from 'node:assert/strict';
import { readFile, readdir } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';

const corpus = new URL('./playground_previews/', import.meta.url);
const imports = new Set([
  'print_i64', 'print_bool', 'print_str', 'print_newline', 'print_f64',
  'actor_spawn', 'actor_send', 'state_register', 'state_enter', 'state_leave'
]);
function gitBlobSha(bytes) {
  return createHash('sha1').update(Buffer.from('blob ' + bytes.length + '\0'))
    .update(bytes).digest('hex');
}

export async function auditPlaygroundPreviews({ compile, runGuest, nativeBytes = null }) {
  const raw = await readFile(new URL('MANIFEST.tsv', corpus), 'utf8');
  const pairs = raw.split('\n').filter(line => line && !line.startsWith('#'))
    .map(line => line.split('\t'));
  assert.equal(pairs.length, 42, 'every public preview has a pinned blob');
  assert.ok(pairs.every(([name, sha]) =>
    /^[a-z-]+$/.test(name) && /^[a-f0-9]{40}$/.test(sha)));
  const names = pairs.map(([name]) => name);
  assert.equal(new Set(names).size, 42, 'no duplicate preview names');
  assert.deepEqual((await readdir(corpus)).filter(n => n.endsWith('.atto')).sort(),
    names.map(name => name + '.atto').sort());
  const outcome = { total: 42, admitted: [], unsupported: [],
    guestExecuted: [], unverified: [], nativeCompared: 0, parityCertified: false,
    executionCoverage: { verified: 0, remaining: 42 } };
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
      'result-option-end': { value: 0n, stdout: '8080\n' },
      'modules': { value: 0n, stdout: '42\n' },
      'callable-body-styles': { value: 0n, stdout: '15\n10\n10\n5\n5\n' },
      'traits-interfaces': { value: 0n, stdout: 'Ada\nhello\n' },
      'diamond-inheritance': { value: 0n, stdout: 'Ada\nhello\nroot\ndiamond resolved\n' },
      'trap': { value: 7n, stdout: '5\nid must be nonnegative\nan ordinary throw was caught\n' },
      'borrow-checking': { value: 63n, stdout: '84\n43\n' },
      'closures-functors': { value: 84n, stdout: '' },
      'lexical-scope-pure': { value: 0n, stdout: '42\n42\n2\n42\n' },
      'state-containers': { value: 0n, stdout: '2\n0\n' },
      'fib-match': { value: 144n, stdout: '0 1 1 2 3 5 8 13 21 34 \n' },
      'guard': { value: 0n, stdout: '42\n' },
      'match-values': { value: 0n, stdout: '2\n2\n' },
    }[name];
    if (expected) {
      const guest = runGuest(result.wasm);
      assert.ok(guest.ok, name + ': guest trap ' + guest.error);
      assert.equal(guest.value, expected.value, name + ': exit value');
      assert.equal(guest.stdout, expected.stdout, name + ': stdout');
      outcome.guestExecuted.push(name);
    } else {
      outcome.unverified.push({ name, reason: 'no independent compiled-guest output oracle' });
    }
  }
  assert.equal(outcome.admitted.length + outcome.unsupported.length, 42);
  assert.deepEqual(outcome.guestExecuted.slice().sort(),
    ['function-pipeline', 'generators', 'result-option', 'trap',
     'borrow-checking', 'lexical-scope-pure', 'state-containers',
     'fib-match', 'guard', 'match-values', 'closures-functors',
     'result-option-end', 'modules', 'callable-body-styles',
     'traits-interfaces', 'diamond-inheritance'].sort(),
    'the sixteen currently compiled preview guests must execute');
  outcome.executionCoverage.verified = outcome.guestExecuted.length;
  outcome.executionCoverage.remaining = 42 - outcome.guestExecuted.length;
  console.log('PLAYGROUND_PREVIEW_EXECUTION_COVERAGE ' + JSON.stringify({
    verified: outcome.guestExecuted, unsupported: outcome.unsupported,
    admittedWithoutOracle: outcome.unverified,
    remaining: outcome.executionCoverage.remaining
  }));
  // Opt-in strict release gate: no unsupported/unexecuted examples permitted.
  if (process.env.ATTO_REQUIRE_ALL_EXAMPLES === '1') {
    assert.equal(outcome.executionCoverage.remaining, 0,
      'published previews without actual guest execution');
    assert.equal(outcome.unsupported.length, 0,
      'unsupported preview sources must become executable before parity release');
  }
  return outcome;
}
