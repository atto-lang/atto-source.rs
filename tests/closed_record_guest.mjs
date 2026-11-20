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
 ['pub fnc main(): int {val Record<{z:bool,a:int}> x=Record.of(z=true,a=40);val Record<{a:int,z:bool}> y=Record.of(a=2,z=false);return x.a+y.a;}',42n,''],
 ['pub fnc main(): int {val Record<{"x,y<>":int,"über":int}> x=Record.of("über":2,"x,y<>":40);return x["x,y<>"]+x["über"];}',42n,''],
 ['pub fnc main(): int {let Record p={y:2,x:40}; return p.x+p.y;}',42n,''],
 ['pub fnc main(): int {let Record h={"content-type":"application/json","über field":40,status_code:2}; stdio.println(h["content-type"]); return h["über field"]+h.status_code;}',42n,'application/json\n'],
 ['pub fnc main(): int {let Record x={z:true,a:40}; let record y={a:41,z:false}; return x.a+y.a-39;}',42n,''],
 ['define struct Point as int x; int y; end pub fnc main(): int {val p=Point{y:2,x:40}; let Record c={x:40,y:2}; return p.x+c.y;}',42n,''],
 ['pub fnc main(): int {val x=Record.of(z=2,a=40);return x.a+x.z;}',42n,''],
 ['pub fnc main(): int {val r=Record.of("x-y":40,ok:2);return r["x-y"]+r.ok;}',42n,''],
 ['pub fnc main(): int {val pair=Tuple.of(10,20);val values=Array.of(1,2,3);return values[0]+pair[1];}',21n,''],
 ['pub fnc main(): int {val t=Tuple.of(40,true,"ok");return t[0];}',40n,''],
 ['pub fnc main(): int {val a=Array.of(20,22);return a[0]+a[1];}',42n,''],
 ['pub fnc main(): int {val l=List.of("Atto","Lima");stdio.println(l[0]);return 42;}',42n,'Atto\n'],
 ['pub fnc main(): int {val xs=List.of(20,22);return xs[0]+xs[1];}',42n,''],
 ['pub fnc main(): int {val m=Map.of("a",40,"b",2);return m["a"]+m["b"];}',42n,''],
 ['pub fnc main(): int {val m=Map.of("a"=>40,"b"=>2);return m["b"]+m["a"];}',42n,''],
 ['pub fnc main(): int {val m=Map.of(true,40,false,2);return m[true]+m[false];}',42n,''],
 ['pub fnc main(): int {let Map<string,int> m={"x"=>42};return m["x"];}',42n,''],
 ['pub fnc main(): int {val pair=Tuple.of(10,20);val values=Array.of(1,2,3);val names=List.of("Atto","Lima");val scores=Map.of("a",10,"b",20);val item=Record.of(name="Atto",count=3);return values[0]+pair[1];}',21n,''],
 ['pub fnc main(): int {val List<int> xs=[];val Map<int,string> m={};return 42;}',42n,''],
 ['pub fnc main(): int {val Array<string> xs=[];return 42;}',42n,''],
 ['pub fnc main(): int {val Record r=(abc="foo",xyz=true,foo=5);return r.foo;}',5n,''],
 ['pub fnc main(): int {val Pair<int,bool> p=5=>true;val Entry<int,bool> e=40=>true;return p.first+e.key-3;}',42n,''],
 ['pub fnc main(): int {val Pair<int,bool> p=Pair.of(40,true);val Entry<int,bool> e=Entry.of(2,false);return p.first+e.key;}',42n,''],
 ['pub fnc main(): int {const m=Map.of_entries<int,bool>(5=>true,8=>false,9=>true);return m[5]?42:0;}',42n,''],
 ['pub fnc main(): int {const m=Map.of_entries<int,bool>(Entry.of(5,true),Entry.of(8,false));return m[8]?0:42;}',42n,''],
 ['pub fnc main(): int {const m=Map.of_entries<string,int>("a"=>40,"b"=>2);return m["a"]+m["b"];}',42n,''],
 ['pub fnc main(): int {const m=Map.of_entries<int,bool>();return 42;}',42n,''],
 ['define struct Point as int x; int y; end define struct Offset as int x; int y; end pub fnc main(): int {val Point p=Point{y:2,x:40};val Offset o=Offset{x:2,y:0};return p.x+o.x;}',42n,'']

];
const negatives=[
 ['pub fnc main(): int {val Record<{a:int,b:bool}> x=Record.of(a=42);return x.a;}','E_NO_FIELD'],
 ['pub fnc main(): int {val Record<{a:int}> x=Record.of(a=42,extra=true);return x.a;}','E_NO_FIELD'],
 ['pub fnc main(): int {val Record<{a:string}> x=Record.of(a=42);return 0;}','E_TYPE'],
 ['pub fnc main(): int {val Record<{items:Array<int>}> x=Record.of(items=Array.of());return 0;}','E_UNSUPPORTED'],
 ['pub fnc main(): int {let Record r={x:42}; let string key="x"; return r[key];}','E_UNSUPPORTED'],
 ['pub fnc main(): int {let Record r={x:42}; return r["missing"]; }','E_NO_FIELD'],
 ['pub fnc main(): int {let Record r={x:1,"x":2}; return 0; }','E_DUPLICATE'],
 ['pub fnc main(): int {const Record r={x:42}; return r.x;}','E_UNSUPPORTED'],
 ['pub fnc main(): int {val r=Record.of(name=1,"name":2);return 0;}','E_DUPLICATE'],
 ['pub fnc main(): int {val m=Map.of("a",1,"b");return 0;}','E_FACTORY_ARITY'],
 ['pub fnc main(): int {val a=Array.of();return 0;}','E_TYPE_INFERENCE'],
 ['pub fnc main(): int {val a=Array.of(1,2);return a[2];}','E_INDEX_BOUNDS'],
 ['pub fnc main(): int {val t=Tuple.of(10);return t[-1];}','E_INDEX_BOUNDS'],
 ['pub fnc main(): int {val t=Tuple.of(10);val i=0;return t[i];}','E_UNSUPPORTED'],
 ['pub fnc main(): int {val a=Array.of(1,2);val writer=borrow! mut a;return 0;}','E_BORROW'],
 ['pub fnc main(): int {val t=Tuple.of();return 0;}','E_UNSUPPORTED'],
 ['pub fnc main(): int {val l=List.of(10,20);return l[2];}','E_INDEX_BOUNDS'],
 ['pub fnc main(): int {val l=List.of(10,20);val writer=borrow! mut l;return 0;}','E_BORROW'],
 ['pub fnc main(): int {val m=Map.of("a",1);return m["missing"];}', 'E_UNSUPPORTED'],
 ['pub fnc main(): int {val m=Map.of("a",1);val writer=borrow! mut m;return 0;}', 'E_BORROW'],
 ['pub fnc main(): int {val m=Map.of(0,1,-0,2);return 0;}', 'E_DUPLICATE'],
 ['pub fnc main(): int {val Tuple[int?,string?,bool?] xs=(None,None,None);return 0;}','E_UNSUPPORTED'],
 ['pub fnc main(): int {val Pair<int,bool> p=Entry.of(5,true);return 0;}','E_TYPE'],
 ['pub fnc main(): int {val Entry<int,bool> e=Pair.of(5,true);return 0;}','E_TYPE'],
 ['pub fnc main(): int {val Pair<int,bool> p=5=>42;return 0;}','E_TYPE'],
 ['pub fnc main(): int {val List<int> xs=[1,"bad"];return 0;}','E_TYPE'],
 ['pub fnc main(): int {val Map<int,string> xs={"a"=>"bad"};return 0;}','E_TYPE'],
 ['pub fnc main(): int {const m=Map.of_entries<int,bool>(5,true);return 0;}','E_FACTORY_ARITY'],
 ['pub fnc main(): int {const m=Map.of_entries<int,bool>(Pair.of(5,true));return 0;}','E_TYPE'],
 ['pub fnc main(): int {const m=Map.of_entries<int,bool>(5=>true,5=>false);return 0;}','E_DUPLICATE'],
 ['pub fnc main(): int {const m=Map.of_entries<int,bool>("5"=>true);return 0;}','E_TYPE'],
 ['pub fnc main(): int {const m=Map.of_entries<float,bool>(5.0=>true);return 0;}','E_MAP_KEY'],
 ['define struct First as int x; end define struct Second as int x; end pub fnc main(): int {val x=infer struct{x:42};return 0;}','E_AMBIGUOUS_MEMBER'],
 ['define struct Point as int x; int y; end pub fnc main(): int {val p=Point{x:42};return 0;}','E_SHAPE']

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
 console.log('PASS canonical factories and closed Record guests: '+positives.length+' executed, '+negatives.length+' rejected with zero artifact, native/browser decisions and byte parity');
} finally {await rm(dir,{recursive:true,force:true})}
