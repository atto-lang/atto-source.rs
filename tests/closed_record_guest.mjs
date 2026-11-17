// Actual guest-Wasm and native compiler parity, no simulated interpreter.
import assert from 'node:assert/strict';
import {mkdtemp,readFile,writeFile,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {spawnSync} from 'node:child_process';
const native=process.env.ATTOC||'target/release/attoc';
const path=process.env.ATTO_WASM||'target/wasm32-unknown-unknown/release/atto_compiler.wasm';
const module=new WebAssembly.Module(await readFile(path));
assert.deepEqual(WebAssembly.Module.imports(module),[]);
const cc=new WebAssembly.Instance(module,{}).exports;
assert.equal(cc.abi_version(),2);
const enc=new TextEncoder(),dec=new TextDecoder();
function browser(source){
 const bytes=enc.encode(source);
 assert.ok(bytes.length<cc.input_capacity());
 new Uint8Array(cc.memory.buffer,cc.input_ptr(),bytes.length).set(bytes);
 const status=cc.compile(bytes.length);
 if(status===0){
  const out=new Uint8Array(cc.memory.buffer,cc.result_ptr(),cc.result_length()).slice();
  assert.deepEqual([...out.slice(0,8)],[0,97,115,109,1,0,0,0]);
  return {ok:true,bytes:out};
 }
 assert.equal(status,1);
 assert.equal(cc.result_length(),0);
 const mem=new Uint8Array(cc.memory.buffer);
 let end=cc.error_ptr();
 while(end<mem.length&&mem[end]!==0) end++;
 assert.ok(end<mem.length);
 return {ok:false,error:dec.decode(mem.slice(cc.error_ptr(),end))};
}
function run(bytes){
 let stdout='',inst;
 const imported={
  print_i64(n){stdout+=String(n)},
  print_bool(n){stdout+=n?'true':'false'},
  print_f64(n){stdout+=String(n)},
  print_str(p,n){stdout+=dec.decode(new Uint8Array(inst.exports.memory.buffer,p,n))},
  print_newline(){stdout+='\n'},
  actor_spawn(){throw Error('actor unadmitted')},
  actor_send(){throw Error('actor unadmitted')}
 };
 const m=new WebAssembly.Module(bytes);
 for(const i of WebAssembly.Module.imports(m)){
  assert.equal(i.module,'atto');
  assert.ok(Object.hasOwn(imported,i.name));
 }
 inst=new WebAssembly.Instance(m,{atto:imported});
 return {value:inst.exports.main(),stdout};
}
const positives=[
 ['pub fnc main(): int {let Record p={y:2,x:40}; return p.x+p.y;}',42n,''],
 ['pub fnc main(): int {let Record h={"content-type":"application/json","über field":40,status_code:2}; stdio.println(h["content-type"]); return h["über field"]+h.status_code;}',42n,'application/json\n'],
 ['pub fnc main(): int {let Record x={z:true,a:40}; let record y={a:41,z:false}; return x.a+y.a-39;}',42n,''],
 ['define struct Point as int x; int y; end pub fnc main(): int {val p=Point{y:2,x:40}; let Record c={x:40,y:2}; return p.x+c.y;}',42n,'']
 ['pub fnc main(): int {let Record headers={status_code:42,"content-type":"ok"}; headers.status_code; headers["content-type"]; return headers.status_code;}',42n,''],
 ['pub fnc main(): int {let Record headers={"content-type":"application/json",status_code:42}; block { stdio.println(headers["content-type"]); headers.status_code; } return headers["status_code"];}',42n,'application/json\n'],
 ['pub fnc main(): int {let Record headers={"content-type":42,status_code:42}; val alias=headers; return alias["content-type"];}',42n,''],
 ['pub fnc main(): int {let Record headers={"status_code":42,"content-type":"ok"}; return headers.status_code+headers["status_code"]-42;}',42n,''],
 ['pub fnc main(): int {let Record headers={"":1,"café":41,"café":7}; return headers[""]+headers["café"];}',42n,''],
];
const negatives=[
 ['pub fnc main(): int {let Record r={x:42}; let string key="x"; return r[key];}','E_UNSUPPORTED'],
 ['pub fnc main(): int {let Record r={x:42}; return r["missing"]; }','E_NO_FIELD'],
 ['pub fnc main(): int {let Map<string,int> m={"x"=>42}; return m["x"]; }','E_UNSUPPORTED'],
 ['pub fnc main(): int {let Record r={x:1,"x":2}; return 0; }','E_DUPLICATE'],
 ['pub fnc main(): int {const Record r={x:42}; return r.x;}','E_UNSUPPORTED']
 ['pub fnc main(): int {let Record headers={"content-type":42}; block {let headers=7; stdio.println(headers["content-type"]);} return 0;}','E_UNSUPPORTED'],
 ['define struct Header as int status_code; end pub fnc main(): int {val h=Header{status_code:42}; return h["status_code"]; }','E_UNSUPPORTED'],
 ['pub fnc main(): int {let Record headers={"content-type":42}; return headers["content"+"-type"]; }','E_UNSUPPORTED'],
 ['pub fnc main(): int {let Record headers={"café":42}; return headers["café"]; }','E_NO_FIELD'],
 ['pub fnc main(): int {let Record headers={"content-type":42}; return headers[0]; }','E_UNSUPPORTED'],
];
const dir=await mkdtemp(join(tmpdir(),'atto-closed-record-'));
try{
 let n=0;
 for(const [source,value,stdout] of positives){
  const file=join(dir,'ok'+(n++)+'.atto');
  await writeFile(file,source);
  const raw=spawnSync(native,[file],{encoding:null,maxBuffer:4*1024*1024});
  assert.equal(raw.error,undefined);
  assert.equal(raw.status,0,String(raw.stderr));
  const compiled=browser(source);
  assert.equal(compiled.ok,true,compiled.error);
  assert.deepEqual(raw.stdout,Buffer.from(compiled.bytes));
  assert.deepEqual(run(compiled.bytes),{value,stdout});
 }
 n=0;
 for(const [source,code] of negatives){
  const file=join(dir,'reject'+(n++)+'.atto');
  await writeFile(file,source);
  const raw=spawnSync(native,[file],{encoding:null,maxBuffer:4*1024*1024});
  assert.equal(raw.error,undefined);
  assert.equal(raw.status,1,String(raw.stderr));
  assert.equal(raw.stdout.length,0);
  assert.ok(String(raw.stderr).includes(code),String(raw.stderr));
  const compiled=browser(source);
  assert.equal(compiled.ok,false,'unsupported guest admitted');
  assert.equal(compiled.error.split(':')[0],code);
 }
 console.log(`PASS typed closed Record dot/bracket guests: ${positives.length} executed, ${negatives.length} rejected with zero artifact, ${positives.length+negatives.length} native/Wasm decisions and byte parity`);
} finally {await rm(dir,{recursive:true,force:true})}
