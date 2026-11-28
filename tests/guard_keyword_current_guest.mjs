import assert from 'node:assert/strict';
import {readFileSync,writeFileSync,existsSync,mkdtempSync,rmSync} from 'node:fs';
import {spawnSync} from 'node:child_process';
import {join} from 'node:path';
import {tmpdir} from 'node:os';
const api=new WebAssembly.Instance(new WebAssembly.Module(readFileSync('target/wasm32-unknown-unknown/release/atto_compiler.wasm')),{}).exports;
const tmp=mkdtempSync(join(tmpdir(),'rust-guard-'));
const good=[
  'pub fnc main():int{guard true else return 1; end return 42;}',
  'pub fnc main():int{guard false else return 42; end return 1;}',
  'pub fnc main():int{if true then guard true {return 1;} else return 2; fi return 42;}',
  'fnc fault(int n):bool{return n+1>0;}pub fnc main():int{guard false,fault(9223372036854775807) else return 42; end return 1;}'
];
const bad=[
  'pub fnc main():int{guard false else val x=1; end return 42;}',
  'pub fnc main():int{guard false else throw Error("bad"); end return 42;}',
  'pub fnc main():int{guard true,3 else return 1; end return 42;}',
  'pub fnc main():int{guard true {return 1;} else return 2; end return 42;}'
];
try {
  for(const [i,source] of [...good,...bad].entries()){
    const path=join(tmp,i+'.atto'),out=join(tmp,i+'.wasm');
    const bytes=Buffer.from(source);writeFileSync(path,bytes);
    assert.ok(bytes.length<api.input_capacity());
    new Uint8Array(api.memory.buffer,api.input_ptr(),bytes.length).set(bytes);
    const rc=api.compile(bytes.length);
    const n=spawnSync('target/release/attoc',[path,'-o',out],{encoding:'utf8',timeout:30000});
    if(i<good.length){
      assert.equal(n.status,0,n.stderr);assert.equal(rc,0);
      const guest=readFileSync(out);
      assert.deepEqual(guest,Buffer.from(new Uint8Array(api.memory.buffer,api.result_ptr(),api.result_length())));
      const g=new WebAssembly.Instance(new WebAssembly.Module(guest),{}).exports;
      assert.equal(g.main(),42n);assert.equal(g.panic_code.value,0);
      console.log('PASS native/browser guest',i);
    }else{
      assert.equal(n.status,1,n.stderr);assert.notEqual(rc,0);
      assert.equal(api.result_length(),0);assert.equal(existsSync(out),false);
      if(i<7)assert.match(n.stderr,new RegExp(['E_GUARD','E_GUARD','E_TYPE'][i-4]));
      console.log('PASS dual no-artifact rejection',i,n.stderr.trim());
    }
  }
}finally{rmSync(tmp,{recursive:true,force:true});}
