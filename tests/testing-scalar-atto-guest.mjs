// Real attoc -> guest WebAssembly execution of backend-local .atto test helpers.
// Unlike source-free parity comparisons, this validates actual generated guest
// return values. Requires an independently built source-pinned attoc binary.
import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {readFileSync, writeFileSync, mkdtempSync, rmSync, existsSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve, dirname} from 'node:path';
import {fileURLToPath} from 'node:url';
import {createHash} from 'node:crypto';
import {SCALAR_TEST_CASES,SCALAR_TEST_NEGATIVES,scalarTestProgram} from './testing-scalar-vectors.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const compiler = process.env.ATTOC ?? resolve(here, '../target/debug/attoc');
assert.ok(existsSync(compiler), 'real attoc compiler binary is required');
const file = resolve(here, '../stdlib/testing_scalar.atto');
const library = readFileSync(file, 'utf8');
const sha256 = value => createHash('sha256').update(value).digest('hex');
const work = mkdtempSync(join(tmpdir(), 'atto-rs-testing-'));
const receipts = [];
try {
  for (const [id, expression, expected] of SCALAR_TEST_CASES) {
    // Native attoc resolves its own built-in .atto source; never a host mock.
    const source = scalarTestProgram(expression);
    const input = join(work, id + '.atto');
    const output = join(work, id + '.wasm');
    writeFileSync(input, source);
    const result = spawnSync(compiler, [input, '-o', output], {encoding: 'utf8', timeout: 30000, maxBuffer: 1 << 20});
    assert.equal(result.error, undefined, id + ': compiler process error');
    assert.equal(result.status, 0, id + ': compiler rejected .atto source: ' + result.stderr);
    assert.ok(existsSync(output), id + ': compiler emitted no guest module');
    const binary = readFileSync(output);
    assert.ok(WebAssembly.validate(binary), id + ': invalid guest WebAssembly');
    const instance = new WebAssembly.Instance(new WebAssembly.Module(binary), {atto: {}});
    assert.equal(typeof instance.exports.main, 'function', id + ': main export absent');
    const actual = instance.exports.main();
    assert.equal(actual, expected, id + ': real guest return mismatch');
    if (instance.exports.panic_code !== undefined) {
      assert.equal(instance.exports.panic_code.value, 0, id + ': guest panicked');
    }
    receipts.push({id, source_sha256: sha256(source), wasm_sha256: sha256(binary), expected: String(expected), actual: String(actual), guest_executed: true});
  }
  const invalid = [
    ['legacy-suffix','import fnc equal_int from "std/testing_scalar.ores"; pub fnc main():int{return equal_int(42,42);}'],
    ['path-traversal','import fnc equal_int from "std/../testing_scalar"; pub fnc main():int{return equal_int(42,42);}'],
    ['unknown-library','import fnc equal_int from "std/unknown"; pub fnc main():int{return equal_int(42,42);}'],
    ['unknown-export','import fnc missing from "std/testing_scalar"; pub fnc main():int{return missing(42,42);}'],
    ['duplicate-binding','import fnc equal_int as foo, not_equal_int as foo from "std/testing_scalar"; pub fnc main():int{return foo(42,42);}'],
    ['collision','import fnc equal_int from "std/testing_scalar"; pub fnc equal_int(int x,int y):int{return 1;} pub fnc main():int{return 0;}'],
    ['relative-file-without-loader','import fnc equal_int from "./stdlib/testing_scalar.atto"; pub fnc main():int{return equal_int(42,42);}'],
    ['forged-internal-namespace','import fnc equal_int from "std/testing_scalar"; pub fnc main():int{return __atto_file_import_0.equal_int(42,42);}'],
  ];
  for(const [id,source] of SCALAR_TEST_NEGATIVES){
    const input=join(work,'reject-'+id+'.atto');
    const output=join(work,'reject-'+id+'.wasm');
    writeFileSync(input,source);
    const result=spawnSync(compiler,[input,'-o',output],{