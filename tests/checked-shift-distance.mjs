// Execute the actual native attoc compiler's generated Wasm (not host shifts).
import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {mkdtempSync,writeFileSync,rmSync,readFileSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
const attoc=process.env.ATTOC;
assert.ok(attoc,'ATTOC must identify the freshly built native compiler');
const dir=mkdtempSync(join(tmpdir(),'atto-checked-shifts-'));
const src=join(dir,'input.atto');
const mask=2n**64n-1n;
const signed=x=>BigInt.asIntN(64,x);
const expect=(a,n,op)=>{
  const d=2n**n;
  if(op==='<<<')return signed(a*d);
  if(op==='>>>')return signed((a&mask)/d);
  const q=a/d;
  return a<0n&&a%d!==0n?q-1n:q;
};
function compile(source){
  writeFileSync(src,source);
  const p=spawnSync(attoc,[src],{timeout:10000,maxBuffer:2**22});
  assert.equal(p.error,undefined);
  assert.equal(p.signal,null);
  return p;
}
function run(p,invalid=false){
  assert.equal(p.status,0,p.stderr.toString());
  const m=new WebAssembly.Module(p.stdout);
  assert.deepEqual(WebAssembly.Module.imports(m),[]);
  const i=new WebAssembly.Instance(m,{});
  if(invalid){
    assert.throws(()=>i.exports.main(),WebAssembly.RuntimeError);
    assert.equal(i.exports.panic_code.value,8);
  }else{
    const result=i.exports.main();
    assert.equal(i.exports.panic_code.value,0);
    return result;
  }
}
const source=(op,a,n)=>'fnc shift(int a,int n): int { return a '+op+' n; }\npub fnc main(): int { return shift(('+a+'), ('+n+')); }\n';
try{
  let checks=0;
  for(const op of ['<<<','>>>','ashr']){
    for(const a of [-(1n<<63n),-9007199254740993n,-9n,-1n,0n,1n,9n,9007199254740993n,(1n<<63n)-1n])
      for(const n of [0n,1n,2n,31n,32n,63n]){
        assert.equal(run(compile(source(op,a,n))),expect(a,n,op),op+' '+a+' '+n);
        checks++;
      }
    for(const n of [-9223372036854775808n,-1n,64n,65n,9223372036854775807n]){
      run(compile(source(op,1n,n)),true);
      const p=compile('pub fnc main(): int { return 1 '+op+' ('+n+'); }\n');
      assert.notEqual(p.status,0);
      assert.equal(p.stdout.length,0);
      assert.match(p.stderr.toString(),/E_SHIFT_DISTANCE/);
      checks+=2;
    }
  }
  for(const file of ['190-bitwise-triple-shifts.atto','191-bitwise-sign-fill.atto']){
    assert.equal(run(compile(readFileSync(new URL('./fixtures/pass/'+file,import.meta.url),'utf8'))),42n);
    checks++;
  }
  // The postpass must still relocate recursive-call offsets when a shift adds
  // a local. This also exercises nested expression operand ordering.
  const recursive='fnc f(int n): int { if n eq 0 then return 1; fi return f(n - 1) <<< 1; }\npub fnc main(): int { return f(5) + 10; }\n';
  assert.equal(run(compile(recursive)),42n);
  checks++;
  console.log('REAL_RUST_WASM_CHECKED_SHIFT_PASS',checks);
}finally{rmSync(dir,{recursive:true,force:true});}
