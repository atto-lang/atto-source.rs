// Real compiler -> Wasm -> guest execution for named scalar generics.
// Run: cargo build --locked --release --bin attoc
//      ATTOC=target/release/attoc node tests/named-scalar-generic-guest.mjs
import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync, readFileSync, rmSync, existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { spawnSync } from 'node:child_process';

const compiler = resolve(process.env.ATTOC || 'target/release/attoc');
const dir = mkdtempSync(join(tmpdir(), 'atto-named-generic-'));
const definition = 'fnc identity<T>(T x): T { return x; } ';
function build(expression) {
  const source = definition + 'pub fnc main(): int { return ' + expression + '; }';
  const output = join(dir, 'guest.wasm');
  rmSync(output, { force: true });
  writeFileSync(join(dir, 'main.atto'), source);
  const r = spawnSync(compiler, ['main.atto', '-o', output], {
    cwd: dir, encoding: 'utf8', timeout: 30000, maxBuffer: 1024 * 1024,
  });
  assert.ifError(r.error);
  assert.equal(r.signal, null);
  return {r, output};
}
try {
  for (const expression of ['identity<T=int>(42)', 'identity<int>(42)', 'identity<T=int>(x:42)',
    'identity<T=int>(20) + identity<int>(22)']) {
    const {r, output} = build(expression);
    assert.equal(r.status, 0, r.stderr);
    const bytes = readFileSync(output);
    assert.ok(WebAssembly.validate(bytes));
    const module = new WebAssembly.Module(bytes);
    assert.deepEqual(WebAssembly.Module.imports(module), []);
    const instance = new WebAssembly.Instance(module, {});
    assert.equal(instance.exports.main(), 42n);
    assert.equal(instance.exports.panic_code.value, 0);
    console.log('PASS guest:', expression);
  }
  for (const expression of ['identity<U=int>(42)', 'identity<t=int>(42)',
    'identity<T=3>(42)', 'identity<T=int,T=int>(42)', 'identity<T=int,bool>(42)',
    'identity[T=int](42)', 'identity<T=int](42)', 'identity<T=int>', 'identity<T=int>(wrong:42)', 'identity<T=int>(x:)', 'identity<T=int>(x:42,x:42)', 'identity<T=int>(x:42,42)', 'identity<T=int>(x:true)']) {
    const {r, output} = build(expression);
    assert.equal(r.status, 1, r.stderr);
    assert.match(r.stderr, /E_[A-Z_]+/);
    assert.equal(existsSync(output), false);
    console.log('PASS rejection:', expression);
  }
} finally {
  rmSync(dir, {recursive: true, force: true});
}
