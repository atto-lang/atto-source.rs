// Compare the actual native CLI and freshly built wasm32 compiler ABI.
import assert from 'node:assert/strict';
import {readFileSync,writeFileSync,mkdtempSync,rmSync} from 'node:fs';
import {spawnSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {tmpdir} from 'node:os';
import {join,resolve} from 'node:path';
const [cliPath,compilerPath,corpusPath]=process.argv.slice(2);
assert.ok(cliPath&&compilerPath&&corpusPath,'native CLI, wasm32 compiler and shared corpus required');
const cli=resolve(cliPath), corpus=resolve(corpusPath);
const manifest=JSON.parse(readFileSync(join(corpus,'manifest.json'),'utf8'));
const fixtures=manifest.cases.filter(c=>/^(counted-|reject-counted-)/.test(c.id));
assert.equal(fixtures.length,17);
assert.equal(fixtures.filter(c=>c.target.mode==='run').length,10);
const compilerModule=new WebAssembly.Module(readFileSync(compilerPath));
assert.deepEqual(WebAssembly.Module.imports(compilerModule),[]);
const {exports:e}=new WebAssembly.Instance(compilerModule);
assert.equal(typeof e.compile_counted_ready_v1,'function');
const temp=mkdtempSync(join(tmpdir(),'atto-counted-'));
function abi(raw,mode=e.compile_counted_ready_v1){
  assert.ok(raw.length<=e.input_capacity());
  new Uint8Array(e.memory.buffer,e.input_ptr(),raw.length).set(raw);
  const status=mode(raw.length);
  const n=e.result_length();
  return {status,bytes:Buffer.from(new Uint8Array(e.memory.buffer,e.result_ptr(),n))};
}
function compile(raw,name,ok){
  const path=join(temp,'guest.atto');
  writeFileSync(path,raw);
  const native=spawnSync(cli,['--counted-ready-v1',path],{timeout:10000});
  assert.ifError(native.error);
  const wasm=abi(raw);
  assert.equal(native.status,ok?0:1,name+' native: '+native.stderr);
  assert.equal(wasm.status,ok?0:1,name+' wasm32 compiler');
  assert.deepEqual(native.stdout,wasm.bytes,name+' native/wasm32 byte parity');
  if(!ok){assert.equal(wasm.bytes.length,0,name);return;}
  const mod=new WebAssembly.Module(wasm.bytes);
  assert.deepEqual(WebAssembly.Module.imports(mod),[],name);
  assert.deepEqual(WebAssembly.Module.exports(mod).map(x=>x.name).sort(),['main','panic_code']);
  return new WebAssembly.Instance(mod).exports;
}
try{
  for(const c of fixtures){
    const raw=readFileSync(join(corpus,c.path));
    assert.equal(createHash('sha1').update('blob '+raw.length+'\0').update(raw).digest('hex'),c.gitBlobSha1,c.id);
    const guest=compile(raw,c.id,c.target.mode==='run');
    if(guest){
      for(let i=0;i<2;i++){assert.equal(guest.main(),42n,c.id);assert.equal(guest.panic_code.value,0,c.id);}
      console.log('GUEST_PASS '+c.id);
    }else console.log('REJECT_PASS '+c.id);
    const ordinary=abi(raw,e.compile);
    assert.equal(ordinary.status,1,c.id+' ordinary profile');
    assert.equal(ordinary.bytes.length,0,c.id+' stale artifact');
  }
  for(const [name,body,code] of [
    ['invalid-count','val int count = 17; val Array[int] b = await select<int>(count) { when readch a: int n -> { return n; } }; return b[0];',8],
    ['empty-read','val Array[int] b = await select<int>(0) { when readch a: int n -> { return n; } }; return readch a;',9],
    ['would-suspend','val Array[int] b = await select<int>(1) { when readch a: int n -> { return n; } }; return b[0];',9],
    ['bounds','writech a, 42; val Array[int] b = await select<int>(1) { when readch a: int n -> { return n; } }; return b[1];',10],
    ['capacity','writech a, 1; writech a, 2; val Array[int] b = await select<int>(0) { when readch a: int n -> { return n; } }; return 0;',9],
  ]){
    const guest=compile(Buffer.from('suspends fnc main(): int { val Channel<int> a = Channel.new<int>(1); '+body+' }'),name,true);
    assert.throws(()=>guest.main(),WebAssembly.RuntimeError,name);
    assert.equal(guest.panic_code.value,code,name);
    console.log('RUNTIME_REJECT_PASS '+name);
  }
  console.log('COUNTED_READY_WASM_PARITY_PASS 10 positive, 7 negative, 5 runtime traps; 17 ordinary profile rejections');
}finally{rmSync(temp,{recursive:true,force:true});}
