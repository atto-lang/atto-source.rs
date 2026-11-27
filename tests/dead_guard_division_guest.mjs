import assert from 'node:assert/strict';
import {readFileSync,writeFileSync,existsSync,mkdtempSync,rmSync} from 'node:fs';
import {spawnSync} from 'node:child_process';
import {join} from 'node:path';
import {tmpdir} from 'node:os';
const api=new WebAssembly.Instance(new WebAssembly.Module(readFileSync('target/wasm32-unknown-unknown/release/atto_compiler.wasm')),{}).exports;
const tmp=mkdtempSync(join(tmpdir(),'rust-dead-guard-'));
const good=[
  'pub fnc main():int{let int zero=0;guard false,42/zero eq 0 else return 42; end return 1;}',
  'pub fnc main():int{guard false,42/0 eq 0{return 42;}return 1;}',
  'pub fnc main():int{guard false,-9223372036854775808/-1 eq 0{return 42;}return 1;}',
  'pub fnc main():int{let int zero=0;return true ? 42 : (false ? 0 : 42/zero);}'
];
const bad=[
  ['pub fnc main():int{let int zero=0;guard true,42/zero eq 0{return 42;}return 1;}','E_UNSUPPORTED'],
  ['pub fnc main():int{let int zero=0;val int n=true ? 42 : 42/zero;return n/zero;}','E_UNSUPPORTED'],
  ['fnc choose(bool b,int d):int{return b ? 42 : 42/d;} pub fnc main():int{return choose(true,0);}','E_UNSUPPORTED'],
  ['pub fnc main():int{return true ? 42 : false/0;}','E_TYPE'],
  ['pub fnc main():int{return true ? 42 : missing/0;}','E_UNBOUND'],
  ['pub fnc main():int{return true ? 42 : (42/0 eq 0);}','E_TYPE']
];
try{
 for(const [i,[source,code]] of [...good.map(s=>[s,null]),...bad].entries()){
   const path=join(tmp,i+'.atto'),out=join(tmp,i+'.wasm');
   const bytes=Buffer.from(source);writeFileSync(path,bytes);
   assert.ok(bytes.length<api.input_capacity());
   new Uint8Array(api.memory.buffer,api.input_ptr(),bytes.length).set(bytes);
   const rc=api.compile(bytes.length);
   const n=spawnSync('target/release/attoc',[path,'-o',out],{encoding:'utf8',timeout:30000});
   if(code===null){
     assert.equal(n.status,0,n.stderr);assert.equal(rc,0);
     const guest=readFileSync(out);
     assert.deepEqual(guest,Buffer.from(new Uint8Array(api.memory.buffer,api.result_ptr(),api.result_length())));
     const g=new WebAssembly.Instance(new WebAssembly.Module(guest),{}).exports;
     assert.equal(g.main(),42n);assert.equal(g.panic_code.value,0);
   }else{
     assert.equal(n.status,1,n.stderr);assert.notEqual(rc,0);
     assert.equal(api.result_length(),0);assert.equal(existsSync(out),false);
     assert.match(n.stderr,new RegExp(code));
   }
   console.log('DEAD_GUARD_DUAL_PASS',i);
 }
}finally{rmSync(tmp,{recursive:true,force:true});}
