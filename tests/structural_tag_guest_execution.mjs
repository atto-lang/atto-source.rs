// Native attoc -> actual Wasm guests, with exact shared fixture bytes.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdtemp, readFile, rm, stat } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const compiler = process.env.ATTOC || fileURLToPath(new URL('../target/debug/attoc', import.meta.url));
const cases = [
  ['same-brand-nominal', '2ce6818fdb2f078d73f466886e542bcb2a6f9104', 42n],
  ['same-brand-cross-nominal', '51cd7f042f51d5d6100ad2a69155a9894851088f', 42n],
  ['reject-wrong-brand', '75327797926a2ba0dba59b8b3fff6719f259d9f9', 'E_MISSING_BRAND'],
  ['reject-unbranded', '04fe1e6b5c14789c6fbc83d50fa155b6a558aba0', 'E_MISSING_BRAND'],
];
const allowed = new Set(['print_i64','print_bool','print_str','print_newline','print_f64','actor_spawn','actor_send']);
const imports = { atto: Object.fromEntries([...allowed].map(name => [name, () => {
  throw Error('unexpected guest host call: ' + name);
}])) };
const work = await mkdtemp(join(tmpdir(), 'atto-brand-'));
try {
  for (const [name, expectedBlob, outcome] of cases) {
    const path = fileURLToPath(new URL('./markers-brands-v1/' + name + '.atto', import.meta.url));
    const source = await readFile(path);
    assert.equal(createHash('sha1').update('blob ' + source.length + '\0').update(source).digest('hex'), expectedBlob);
    const output = join(work, name + '.wasm');
    const built = spawnSync(compiler, [path, '-o', output], { encoding:'utf8', timeout:30_000, maxBuffer:1<<20 });
    assert.ifError(built.error);
    if (typeof outcome === 'bigint') {
      assert.equal(built.status, 0, built.stderr);
      const bytes = await readFile(output);
      assert.ok(WebAssembly.validate(bytes));
      const module = new WebAssembly.Module(bytes);
      for (const imp of WebAssembly.Module.imports(module)) {
        assert.equal(imp.module, 'atto');
        assert.ok(allowed.has(imp.name));
      }
      const guest = new WebAssembly.Instance(module, imports);
      assert.equal(guest.exports.main(), outcome);
      assert.equal(guest.exports.panic_code.value, 0);
      assert.equal(guest.exports.read_user, undefined, 'scalar helper must not be publicly callable');
      console.log('PASS compiled guest', name, String(outcome), 'blob', expectedBlob);
    } else {
      assert.equal(built.status, 1);
      assert.ok(built.stderr.includes(outcome + ':'), built.stderr);
      await assert.rejects(stat(output), {code:'ENOENT'});
      console.log('PASS no-artifact rejection', name, outcome, 'blob', expectedBlob);
    }
  }
} finally {
  await rm(work, {recursive:true, force:true});
}
