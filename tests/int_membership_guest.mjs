// Real compiler-to-Wasm candidate; no host-side matching oracle.
import assert from 'node:assert/strict';
import {mkdtemp, readFile, writeFile, rm, stat} from 'node:fs/promises';
import {spawnSync} from 'node:child_process';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
const attoc = process.env.ATTOC || 'target/release/attoc';
const program = (subject, arms) => 'pub fnc main(): int { return match ' + subject + ' over ' + arms + ' end; }';
const pass = [
  'const Pattern p = on 5; '+program('5','on p -> 42, on _ -> 0'),
  'const Pattern p = in [5,6,7]; '+program('6','on p -> 42, on _ -> 0'),
  'const Pattern[] z = [on 1,in [2,3],on true]; '+program('3','in z -> 42, on _ -> 0'),
  'const Pattern[] z = [on 1,on true]; '+program('true','in z -> 42, on _ -> 0'),
  'const Pattern[] z = [is int,is bool]; '+program('7','is in z -> 42'),
  program('3','in [on 1,in [2,3]] -> 42, on _ -> 0'),
  'const Pattern p = is Int; '+program('7','on p -> 42'),
  'const Pattern[] z = [on true,on false]; '+program('false','in z -> 42'),
  'const Pattern[] z = [on 1,on 2]; '+program('2','in z when false -> 0, in z -> 42, on _ -> 0'),
  program('2','in [0,1,2] -> 42, on _ -> 0'),
  program('7','in [0,1,2] -> 0, on _ -> 42'),
  program('7','is in [int, string] -> 42'),
  program('true','is in [int, bool] -> 42'),
  program('2','on 0 -> 0, is in [int, bool] -> 42'),
  program('7','in [0,1,2] when (4 / 2) eq 2 -> 0, on _ -> 42'),
  program('2','on 2 -> 42, in [1,2,3] -> 0, on _ -> 0'),
  program('2','in [1,2] when false -> 0, in [1,2] -> 42, on _ -> 0'),
  program('-9223372036854775808','in [-9223372036854775808, 0] -> 42, on _ -> 0'),
  'fnc get(): int { return 2; } ' + program('get()','in [1,2] -> 42, on _ -> 0')
];
const fail = [
  'const Pattern p = on 5; pub fnc main(): int { return p; }',
  'const Pattern p = on q; const Pattern q = on p; '+program('2','on _ -> 42'),
  'const Pattern[] z = [on 1,2]; '+program('2','on _ -> 42'),
  'const Pattern[] z = [is int,on 1]; '+program('2','is in z -> 42'),
  'const Pattern[] z = [on 1,on 1]; '+program('2','on _ -> 42'),
  'const Pattern[] z = [is int,is Int]; '+program('2','on _ -> 42'),
  'const Pattern p = on 1; '+program('1','in p -> 42, on _ -> 0'),
  'const Pattern[] z = [on 1]; '+program('1','on z -> 42, on _ -> 0'),
  'const Pattern p = on 1; fnc f(Pattern x): int { return 42; } pub fnc main(): int { return f(p); }',
  'const Pattern[] z = [on 1,on 2]; '+program('2','in z -> 42, on 2 -> 0, on _ -> 0'),
  // A guard is still statically checked even when its pattern cannot match.
  program('7','in [0,1,2] when (1 / 0) eq 1 -> 0, on _ -> 42'),
  program('2','in [2,2] -> 42, on _ -> 0'),
  program('7','is in [int,int] -> 42'),
  program('7','is in [string,String] -> 42'),
  program('7','is in [] -> 42'),
  program('7','is in [int,42] -> 42'),
  program('7','is in [int,bool] -> 42, on _ -> 0'),
  program('7','is in [string,bool] -> 42, on _ -> 0'),
  program('2','in [] -> 42, on _ -> 0'),
  program('2','in [1,2] -> 42, on 2 -> 0, on _ -> 0'),
  program('2','in [1,2] -> 42'),
  program('true','in [1,2] -> 42, on _ -> 0'),
  program('2','in values -> 42, on _ -> 0'),
  program('2','in [1,true] -> 42, on _ -> 0')
];
const dir = await mkdtemp(join(tmpdir(),'atto-in-'));
try {
  for (const [i, source] of [...pass,...fail].entries()) {
    const input = join(dir, i+'.atto'), output = join(dir, i+'.wasm');
    await writeFile(input,source);
    const built = spawnSync(attoc,[input,'-o',output],{encoding:'utf8',timeout:30000});
    assert.ifError(built.error); assert.equal(built.signal,null);
    if (i < pass.length) {
      assert.equal(built.status,0,built.stderr);
      const bytes = await readFile(output);
      assert.ok(WebAssembly.validate(bytes));
      const module = new WebAssembly.Module(bytes);
      const trap = () => { throw Error('unexpected host call'); };
      const imports = {atto:{print_i64:trap,print_bool:trap,print_str:trap,print_newline:trap,print_f64:trap,actor_spawn:trap,actor_send:trap}};
      const guest = new WebAssembly.Instance(module,imports);
      assert.equal(guest.exports.main(),42n);
      assert.equal(guest.exports.panic_code.value,0);
    } else {
      assert.equal(built.status,1,built.stderr);
      await assert.rejects(stat(output),{code:'ENOENT'});
    }
  }
} finally { await rm(dir,{recursive:true,force:true}); }
console.log('membership guest cases: '+pass.length+' positive, '+fail.length+' negative');
