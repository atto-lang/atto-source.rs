// Executed compiled Atto source guest test: genuine Wasm, not parse-only.
import assert from 'node:assert/strict';
import {mkdtempSync, writeFileSync, readFileSync, rmSync, symlinkSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {spawnSync} from 'node:child_process';

const compiler = resolve(process.env.ATTOC || 'target/release/attoc');
const temp = mkdtempSync(join(tmpdir(), 'atto-real-import-'));
const external = mkdtempSync(join(tmpdir(), 'atto-real-import-external-'));
const math = 'fnc twice(int x): int {return x+x;} pub fnc square(int x): int {return twice(x) + x*(x-2);}';
function compile(main, name='math.atto') {
  writeFileSync(join(temp, 'main.atto'), main);
  writeFileSync(join(temp, 'math.atto'), math);
  const result = spawnSync(compiler, ['main.atto', '-o', 'main.wasm'], {
    cwd: temp, encoding: 'utf8', timeout: 30000,
  });
  return result;
}
function execute(main) {
  const result = compile(main);
  assert.equal(result.status, 0, result.stderr);
  const bytes = readFileSync(join(temp, 'main.wasm'));
  assert.ok(WebAssembly.validate(bytes), 'real Wasm output');
  const mod = new WebAssembly.Module(bytes);
  assert.deepEqual(WebAssembly.Module.imports(mod), []);
  const inst = new WebAssembly.Instance(mod, {atto: {}});
  assert.equal(inst.exports.main(), 49n);
  assert.equal(inst.exports.panic_code.value, 0);
}
try {
  execute('import square from "./math.atto"; pub fnc main(): int {return square(7);}');
  execute('import square as calc from "./math.atto"; pub fnc main(): int {return calc(7);}');
  execute('import * as math from "./math.atto"; pub fnc main(): int {return math.square(7);}');

  const privateUse = compile('import twice from "./math.atto"; pub fnc main(): int {return twice(7);}');
  assert.equal(privateUse.status, 1);
  assert.match(privateUse.stderr, /E_PRIVATE/);

  const invalid = compile('import square from "../secret.atto"; pub fnc main(): int {return square(7);}');
  assert.equal(invalid.status, 1);
  assert.match(invalid.stderr, /E_IMPORT_PATH/);

  writeFileSync(join(external, 'outside.atto'), math);
  symlinkSync(join(external, 'outside.atto'), join(temp, 'escape.atto'));
  const escaped = compile('import square from "./escape.atto"; pub fnc main(): int {return square(7);}');
  assert.equal(escaped.status, 1);
  assert.match(escaped.stderr, /E_IMPORT/);

  const unsupported = compile('import square from pkg "core.telemetry"; pub fnc main(): int {return square(7);}');
  assert.equal(unsupported.status, 1);
  assert.match(unsupported.stderr, /E_UNSUPPORTED/);
  console.log('PASS real compiled Wasm import: 3 positive guest executions, 4 negative fail-closed checks');
} finally {
  rmSync(temp, {recursive: true, force: true});
  rmSync(external, {recursive: true, force: true});
}
