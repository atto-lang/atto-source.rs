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

const here = dirname(fileURLToPath(import.meta.url));
const compiler = process.env.ATTOC ?? resolve(here, '../target/debug/attoc');
assert.ok(existsSync(compiler), 'real attoc compiler binary is required');
const file = resolve(here, '../stdlib/testing_scalar.atto');
const library = readFileSync(file, 'utf8');
const cases = [
  ['equal-pass', 'equal_int(42, 42)', 0n],
  ['equal-fail', 'equal_int(41, 42)', 1n],
  ['not-equal-pass', 'not_equal_int(41, 42)', 0n],
  ['not-equal-fail', 'not_equal_int(42, 42)', 1n],
  ['greater-pass', 'greater_than_int(42, 41)', 0n],
  ['greater-equal-fail', 'greater_than_int(42, 42)', 1n],
  ['less-pass', 'less_than_int(41, 42)', 0n],
  ['less-equal-fail', 'less_than_int(42, 42)', 1n],
  ['range-lower', 'in_range_int(40, 40, 42)', 0n],
  ['range-upper', 'in_range_int(42, 40, 42)', 0n],
  ['range-below', 'in_range_int(39, 40, 42)', 1n],
  ['range-above', 'in_range_int(43, 40, 42)', 1n],
  ['range-inverted', 'in_range_int(41, 42, 40)', 1n],
  ['range-i64-min', 'in_range_int(-9223372036854775808, -9223372036854775808, 9223372036854775807)', 0n],
  ['range-i64-max', 'in_range_int(9223372036854775807, -9223372036854775808, 9223372036854775807)', 0n],
  ['combined-pass', 'combine_status(equal_int(42, 42), less_than_int(41, 42))', 0n],
  ['combined-fail', 'combine_status(equal_int(42, 42), less_than_int(42, 41))', 1n],
];
const sha256 = value => createHash('sha256').update(value).digest('hex');
const work = mkdtempSync(join(tmpdir(), 'atto-rs-testing-'));
const receipts = [];
try {
  for (const [id, expression, expected] of cases) {
    // Import is resolved inside native attoc through its compiler-owned
    // embed, not through concatenation or host filesystem fallback.
    const source = 'import fnc equal_int, not_equal_int, greater_than_int, less_than_int, ' +
      'in_range_int, combine_status from "std/testing_scalar";\n' +
      'pub fnc main(): int { return ' + expression + '; }\n';
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
  for(const [id,source] of invalid){
    const input=join(work,'reject-'+id+'.atto');
    const output=join(work,'reject-'+id+'.wasm');
    writeFileSync(input,source);
    const result=spawnSync(compiler,[input,'-o',output],{encoding:'utf8',timeout:30000,maxBuffer:1<<20});
    assert.equal(result.error,undefined,id+': source compiler process did not execute');
    assert.ok(result.status!==0&&result.status!==null,id+': unsupported import compiled');
    assert.equal(existsSync(output),false,id+': rejected import published a guest artifact');
    assert.ok(result.stderr.includes('E_'),id+': missing classified compiler diagnostic: '+result.stderr);
    receipts.push({id,source_sha256:sha256(source),rejected:true,no_guest_artifact:true});
  }
  console.log(JSON.stringify({schema:'atto.testing-scalar.rs-guest.v1', library_sha256:sha256(library), compiler_sha256:sha256(readFileSync(compiler)), results:receipts}));
} finally {
  rmSync(work, {recursive:true, force:true});
}
