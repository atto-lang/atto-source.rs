import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync, readFileSync, rmSync, existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { spawnSync } from 'node:child_process';
const compiler=resolve(process.env.ATTOC||'target/release/attoc');
const work=mkdtempSync(join(tmpdir(),'atto-primitive-array-'));
const source=(type,values,index)=>'pub fnc main(): int { val '+type+' a = '+values+'; return a['+index+']; }';
function compile(s){
  const path=join(work,'guest.atto'),out=join(work,'guest.wasm');
  rmSync(out,{force:true});writeFileSync(path,s);
  const r=spawnSync(compiler,[path,'-o',out],{encoding:'utf8',timeout:30000,maxBuffer:1048576});
  assert.ifError(r.error);assert.equal(r.signal,null);return {r,out};
}
try {
  let canonical;
  for(const type of ['Array[int]','Array<int>','Array<T=int,layout=aos>']){
    const {r,out}=compile(source(type,'[10,42,-1]','1'));
    assert.equal(r.status,0,r.stderr);const bytes=readFileSync(out);
    if(canonical)assert.deepEqual(bytes,canonical);else canonical=bytes;
    const module=new WebAssembly.Module(bytes);assert.deepEqual(WebAssembly.Module.imports(module),[]);
    const e=new WebAssembly.Instance(module,{}).exports;
    assert.equal(e.main(),42n);assert.equal(e.panic_code.value,0);
    console.log('PRIMITIVE_ARRAY_PASS '+type);
  }
  for(const [values,index,want] of [['[42]','0',42n],['[-9223372036854775808,7]','0',-9223372036854775808n],['[1,2,3]','2',3n]]){
    const {r,out}=compile(source('Array[int]',values,index));assert.equal(r.status,0,r.stderr);
    const e=new WebAssembly.Instance(new WebAssembly.Module(readFileSync(out)),{}).exports;
    assert.equal(e.main(),want);assert.equal(e.panic_code.value,0);
    console.log('PRIMITIVE_ARRAY_PASS '+values+' '+index);
  }
  for(const s of [source('Array[int]','[42]','1'),source('Array[int]','[42]','-1'),source('Array[int]','[]','0'),
    source('Array[int]','[true]','0'),source('Array[int]','[9223372036854775808]','0'),source('Array[int]','[-9223372036854775809]','0'),source('Array[int]','[42]','true'),source('Array[T=int]','[42]','0'),
    source('Array<int,layout=soa>','[42]','0'),source('Array[int]','[42]','0').replace('return a','return other'),
    source('Array[int]','[42]','0').replace('return a','a[0] = 1; return a'),
    source('Array[int]','['+Array(1025).fill(1).join(',')+']','0')]){
    const {r,out}=compile(s);assert.equal(r.status,1,r.stderr);assert.match(r.stderr,/E_[A-Z_]+/);assert.equal(existsSync(out),false);
    console.log('PRIMITIVE_ARRAY_REJECT '+r.stderr.slice(0,160));
  }
  console.log('PRIMITIVE_ARRAY_VERIFIED 18 cases');
}finally{rmSync(work,{recursive:true,force:true})}
