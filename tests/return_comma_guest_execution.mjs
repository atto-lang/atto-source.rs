// Exact-head native Atto compiler -> real Wasm guest execution admission.
// No mock or parser-only pass counts as an execution receipt.
// Run with ATTOC=/absolute/path/to/attoc node tests/return_comma_guest_execution.mjs
import assert from 'node:assert/strict';
import {mkdtemp, readFile, rm, writeFile, stat} from 'node:fs/promises';
import {spawnSync} from 'node:child_process';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {createHash} from 'node:crypto';

const compiler = process.env.ATTOC;
assert.ok(compiler, 'ATTOC must name the compiler built from the exact PR head');
const runner = resolve(compiler);
const cases = [
  ['bare-two', 'return 1, 42;', 42n],
  ['bare-three', 'return 1, 2, 42;', 42n],
  ['caret-bare', '^ 1, 2, 42;', 42n],
  ['explicit-seq', 'return seq 1, 2, 42;', 42n],
  ['contextual-caret', 'return ^ (1, 2, 42);', 42n],
  ['delimited-expr', 'return (> 1, 2), 42;', 42n],
  ['grouped-tail', 'return (40), (40 + 2);', 42n],
  ['nested-args', 'return add(10, 20), add(20, 22);', 42n,
    'fnc add(int a, int b): int { return a + b; }'],
  ['single-return', 'return 42;', 42n],
  ['single-caret', '^ 42;', 42n],
  ['erlang-body', 'return body_result();', 42n,
    'fnc body_result() -> int :: const x: int = 40, x + 2 end'],
];
const invalid = [
  ['empty-caret-sequence', 'return ^ ();', 'E_EMPTY_SEQUENCE'],
  ['trailing-comma', 'return 1,;', 'E_SYNTAX'],
  ['double-comma', 'return 1,,42;', 'E_SYNTAX'],
  ['bad-final-type', 'return 1, true;', 'E_TYPE'],
  ['bare-caret-not-expression', 'return ^ 42;', 'E_SYNTAX'],
  ['ordinary-tuple-not-sequence', 'return (1, 42);', 'E_UNSUPPORTED'],
];
const permittedImports = new Set(['print_i64','print_bool','print_str','print_newline','print_f64',
  'actor_spawn','actor_send','state_register','state_enter','state_leave']);
const work=await mkdtemp(join(tmpdir(),'atto-return-comma-'));
const receipt=[];
function compilerResult(path,dest) {
  const out=spawnSync(runner,[path,'-o',dest], {encoding:'utf8',timeout:30_000,maxBuffer:1<<20});
  assert.ok(!out.error, 'compiler launch failure: '+String(out.error));
  return out;
}
try {
  for(const [id,statement,want,extra=''] of cases) {
    const source=`${extra}\npub fnc main(): int { ${statement} }\n`;
    const path=join(work,id+'.atto'), guestPath=join(work,id+'.wasm');
    await writeFile(path,source,'utf8');
    const result=compilerResult(path,guestPath);
    assert.equal(result.status,0,id+' native compiler error: '+result.stderr);
    const wasm=await readFile(guestPath);
    assert.ok(WebAssembly.validate(wasm),id+' invalid guest Wasm');
    const module=new WebAssembly.Module(wasm);
    const calls=[];
    const atto={};
    for(const imp of WebAssembly.Module.imports(module)){
      assert.equal(imp.module,'atto',id+' unrecognized host import namespace');
      assert.ok(imp.kind==='function'&&permittedImports.has(imp.name),
        id+' unauthorized host import '+imp.name);
      atto[imp.name]=(...args)=>{ calls.push([imp.name,...args]); return 0; };
    }
    const instance=new WebAssembly.Instance(module,{atto});
    assert.equal(typeof instance.exports.main,'function',id+' missing guest main');
    const actual=instance.exports.main();
    assert.equal(actual,want,id+' wrong last-value return');
    assert.equal(instance.exports.panic_code?.value,0,id+' guest panicked');
    assert.deepEqual(calls,[],id+' unexpected runtime side effects');
    receipt.push({id,result:'passed',want:String(want),
      guest_sha256:createHash('sha256').update(wasm).digest('hex')});
  }
  for(const [id,statement,diagnostic] of invalid){
    const source=`pub fnc main(): int { ${statement} }\n`;
    const path=join(work,id+'.atto'),guestPath=join(work,id+'.wasm');
    await writeFile(path,source,'utf8');
    const result=compilerResult(path,guestPath);
    assert.equal(result.status,1,id+' invalid source unexpectedly accepted: '+result.stderr);
    assert.ok(result.stderr.includes(diagnostic),
      id+' missing diagnostic '+diagnostic+': '+result.stderr);
    await assert.rejects(stat(guestPath),{code:'ENOENT'},id+' emitted invalid artifact');
    receipt.push({id,result:'rejected',diagnostic});
  }
  // Side-effect ordering is tested with the real host import, never simulated.
  const source='pub fnc main(): int { return stdio.println(1), stdio.println(2), 42; }\n';
  const path=join(work,'ordering.atto'),guestPath=join(work,'ordering.wasm');
  await writeFile(path,source,'utf8');
  const built=compilerResult(path,guestPath);
  assert.equal(built.status,0,built.stderr);
  const wasm=await readFile(guestPath);
  assert.ok(WebAssembly.validate(wasm),'invalid ordering guest');
  const mod=new WebAssembly.Module(wasm);
  const events=[];
  const atto={};
  for(const imp of WebAssembly.Module.imports(mod)){
    assert.equal(imp.module,'atto');
    assert.ok(imp.kind==='function'&&permittedImports.has(imp.name));
    atto[imp.name]=(...args)=>{events.push([imp.name,...args]);return 0;};
  }
  const value=new WebAssembly.Instance(mod,{atto}).exports.main();
  assert.equal(value,42n,'side-effect prefix corrupted tail');
  assert.deepEqual(events.filter(x=>x[0]==='print_i64').map(x=>x[1]),[1n,2n],
    'prefix effects were reordered, duplicated or eliminated');
  receipt.push({id:'ordering',result:'passed',effects:'print_i64:1,2'});
  console.log(JSON.stringify({status:'passed',cases:receipt,compiler:runner},null,2));
} finally {
  await rm(work,{recursive:true,force:true});
}
