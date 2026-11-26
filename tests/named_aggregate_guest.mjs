// Real Rust compiler CLI -> Wasm -> WebAssembly engine (not a host model).
import assert from 'node:assert/strict';
import {mkdtemp,writeFile,readFile,stat,rm} from 'node:fs/promises';
import {spawnSync} from 'node:child_process';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {fileURLToPath} from 'node:url';

const attoc=process.env.ATTOC||fileURLToPath(new URL('../target/debug/attoc',import.meta.url));
const cases=[
  [`const Record<infer T1> r=(a=true,b="xyz",c=5);
    const Tuple<infer T2> t=(true,"xyz",5);
    const struct<infer T3> s={a:true,b:"xyz",c:5};
    const Record<T1> r2=(c=10,b="abc",a=false);
    const Tuple<T2> t2=(false,"abc",12);
    const struct<T3> s2={c:20,b:"abc",a:false};
    return r2.c+t2[2]+s2.c;`,42n],
  ['const Record r=(a=42); return r.a;',42n],
  ['const Record<infer R> r=(a=-1); const Record<R> s=(a=43); return r.a+s.a;',42n],
];
const rejects=[
  ['const Record<infer> r=(a=1); return 0;','E_TYPE_NAME'],
  ['const Record<infer R> r=(a=1); const Record<R> s=(a=1,b=2); return 0;','E_TYPE_MISMATCH'],
  ['const Tuple<infer T> t=(1,true); const Tuple<T> s=(true,1); return 0;','E_TYPE_MISMATCH'],
  ['const struct<infer S> s={a:1}; const Record<S> r=(a=1); return 0;','E_AGGREGATE_KIND'],
];
const dir=await mkdtemp(join(tmpdir(),'atto-named-aggregate-'));
try{
  for(const [i,[body,expected]] of [...cases,...rejects].entries()){
    const source=join(dir,i+'.atto'), output=join(dir,i+'.wasm');
    await writeFile(source,'pub fnc main(): int { '+body+' }');
    const built=spawnSync(attoc,[source,'-o',output],{encoding:'utf8',timeout:30000});
    if(typeof expected==='bigint'){
      assert.equal(built.status,0,built.stderr);
      const wasm=await readFile(output);
      assert.ok(WebAssembly.validate(wasm));
      const {instance}=await WebAssembly.instantiate(wasm,{atto:{}});
      assert.equal(instance.exports.main(),expected);
      assert.equal(instance.exports.panic_code.value,0);
    }else{
      assert.notEqual(built.status,0);
      assert.ok(built.stderr.includes(expected),built.stderr);
      await assert.rejects(stat(output),{code:'ENOENT'});
    }
  }
  console.log('PASS named aggregate compiled Wasm guests and fail-closed diagnostics');
}finally{await rm(dir,{recursive:true,force:true});}
