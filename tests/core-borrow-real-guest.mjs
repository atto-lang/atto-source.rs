// REAL Atto -> native attoc -> Wasm guest execution, not a host oracle.
// This intentionally fails if the backend lacks verified typed borrow checking.
// A script result is candidate CI evidence, not an independently attested PASS.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync, rmSync, statSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const compiler = process.env.ATTOC ?? resolve(root, 'target/debug/attoc');
const sha256 = input => createHash('sha256').update(input).digest('hex');
const gitBlob = bytes => createHash('sha1')
  .update(Buffer.from('blob ' + bytes.length + '\0'))
  .update(bytes).digest('hex');

assert.ok(existsSync(compiler), 'real native attoc must be built before guest tests: ' + compiler);
const compilerStat = statSync(compiler);
assert.ok(compilerStat.isFile() && compilerStat.size > 1024,
  'refuse stub or missing compiler binary');
const compilerDigest = sha256(readFileSync(compiler));
const manifest = JSON.parse(readFileSync(
  resolve(root, 'tests/fixtures/core-borrow-v1/manifest.json'), 'utf8'));
assert.equal(manifest.schema, 'atto.core.borrow-checker-guest-candidate-v1');
assert.equal(manifest.cases.length, 6);
assert.equal(manifest.status, 'SOURCE_ONLY_NO_COMPILER_EXECUTION');

const allowedAttoImports = Object.freeze([
  'print_i64', 'print_bool', 'print_f64', 'print_str', 'print_newline',
  'actor_spawn', 'actor_send', 'state_register', 'state_enter', 'state_leave',
]);
const denyHostEffect = () => { throw Error('unexpected host effect from ownership guest'); };
const imports = { atto: Object.fromEntries(allowedAttoImports.map(name => [name, denyHostEffect])) };
const runCompiler = (input, output) => spawnSync(compiler, [input, '-o', output], {
  cwd: root, shell: false, encoding: 'utf8', timeout: 30_000, maxBuffer: 1 << 20,
});
const temp = mkdtempSync(join(tmpdir(), 'atto-core-borrow-real-'));
let passed = 0, rejected = 0;
const receipts = [];
try {
  for (const fixture of manifest.cases) {
    assert.match(fixture.path, /^tests\/fixtures\/core-borrow-v1\/(pass|fail)\/[a-z-]+\.atto$/);
    assert.equal(fixture.status, 'NOT_EXECUTED', 'manifest must not inject a PASS receipt');
    const source = resolve(root, fixture.path);
    const bytes = readFileSync(source);
    assert.equal(gitBlob(bytes), fixture.git_blob_sha, 'fixture blob identity changed');
    const output = join(temp, String(passed + rejected) + '.wasm');
    assert.equal(existsSync(output), false, 'guest artifact preexists');
    const result = runCompiler(source, output);
    assert.equal(result.error, undefined, fixture.path + ': compiler process error');
    assert.equal(result.signal, null, fixture.path + ': compiler killed/timed out');
    if (fixture.expected === 'EXECUTE_GUEST_EXIT_42') {
      assert.equal(result.status, 0, fixture.path + ': compilation failed: ' + result.stderr);
      assert.equal(existsSync(output), true, fixture.path + ': compiled artifact missing');
      const wasm = readFileSync(output);
      assert.ok(WebAssembly.validate(wasm), fixture.path + ': invalid Wasm');
      const mod = new WebAssembly.Module(wasm);
      for (const item of WebAssembly.Module.imports(mod)) {
        assert.equal(item.module, 'atto', 'unapproved host import module');
        assert.ok(Object.hasOwn(imports.atto, item.name),
          fixture.path + ': unapproved host import ' + item.name);
      }
      const instance = new WebAssembly.Instance(mod, imports);
      assert.equal(typeof instance.exports.main, 'function', 'missing compiled guest main');
      assert.equal(instance.exports.main(), 42n, fixture.path + ': incorrect actual guest exit');
      assert.equal(instance.exports.panic_code?.value, 0, fixture.path + ': unexpected guest panic');
      receipts.push({case: fixture.path, kind: 'COMPILED_WASM_GUEST_CANDIDATE',
        source_git_blob_sha: fixture.git_blob_sha, wasm_sha256: sha256(wasm),
        guest_exit: 42, panic_code: 0, executed: true});
      passed++;
    } else {
      assert.equal(fixture.expected, 'REJECT_BEFORE_CODEGEN');
      assert.match(fixture.expected_error, /^E_[A-Z_]+$/);
      assert.equal(result.status, 1, fixture.path + ': invalid program not rejected');
      assert.equal(existsSync(output), false,
        fixture.path + ': invalid source emitted a guest artifact');
      assert.match(result.stderr, new RegExp('\\b' + fixture.expected_error + '\\b'),
        fixture.path + ': wrong compiler diagnostic: ' + result.stderr);
      assert.doesNotMatch(result.stderr, /\bE_(SYNTAX|UNSUPPORTED|UNKNOWN_NAME)\b/,
        fixture.path + ': parser/unsupported fallback masquerades as borrow proof');
      receipts.push({case: fixture.path, kind: 'COMPILER_TYPED_NEGATIVE_CANDIDATE',
        source_git_blob_sha: fixture.git_blob_sha, diagnostic: fixture.expected_error,
        artifact_absent: true, executed: true});
      rejected++;
    }
  }
  assert.equal(passed, 2);
  assert.equal(rejected, 4);
  console.log(JSON.stringify({schema: 'atto.core.borrow.real-guest-candidate-v1',
    status: 'CANDIDATE_EXTERNAL_PROVENANCE_REQUIRED',
    compiler_binary_sha256: compilerDigest, compiler_source_head_claim:
      process.env.ATTO_COMPILER_SOURCE_SHA ?? null,
    positive_guest_count: passed, typed_negative_count: rejected,
    cases: receipts}, null, 2));
} finally {
  rmSync(temp, {recursive: true, force: true});
}
