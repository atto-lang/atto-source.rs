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
 ['pub fnc main(): int {const name="Alice"; const age=42; val z=(:name,:age); stdio.println(z.name); return z["age"];}',42n,'Alice\n'],
 ['pub fnc main(): int {const name="Alice"; const age=42; val z={foo:5,:name,:age}; return z.foo+z.age-5;}',42n,''],
 ['pub fnc main(): int {const name="Alice"; const age=42; val z=(foo=5,:name,:age); return z.foo+z.age-5;}',42n,''],
 ['pub fnc main(): int {const name="Alice"; const age=42; val z={:name,:age}; stdio.println(z.name); return z["age"];}',42n,'Alice\n'],
 ['pub fnc main(): int {val age=40; val z={:age,offset:2}; return z.age+z.offset;}',42n,''],
 ['pub fnc main(): int {let Record p={y:2,x:40}; return p.x+p.y;}',42n,''],
 ['pub fnc main(): int {let Record h={content_type:"application/json",über_field:40,status_code:2}; stdio.println(h["content_type"]); return h["über_field"]+h.status_code;}',42n,'application/json\n'],
 ['pub fnc main(): int {let Record x={z:true,a:40}; let record y={a:41,z:false}; return x.a+y.a-39;}',42n,''],
 ['define struct Point as int x; int y; end pub fnc main(): int {val p=Point{y:2,x:40}; let Record c={x:40,y:2}; return p.x+c.y;}',42n,''],
 ['pub fnc main(): int {let Record headers={status_code:42,content_type:"ok"}; headers.status_code; headers["content_type"]; return headers.status_code;}',42n,''],
 ['pub fnc main(): int {let Record headers={content_type:"application/json",status_code:42}; block { stdio.println(headers["content_type"]); headers.status_code; } return headers["status_code"];}',42n,'application/json\n'],
 ['pub fnc main(): int {let Record headers={content_type:42,status_code:42}; val alias=headers; return alias["content_type"];}',42n,''],
 ['pub fnc main(): int {let Record headers={status_code:42,content_type:"ok"}; return headers.status_code+headers["status_code"]-42;}',42n,''],
 ['pub fnc main(): int {let Record headers={empty:1,café:41,cafe_decomp:7}; return headers["empty"]+headers["café"];}',42n,''],
];
const negatives=[
 ['pub fnc main(): int {const age=42; val z=(:age,age=5); return 0;}','E_DUPLICATE'],
 ['pub fnc main(): int {const age=42; val z=(:age,5); return 0;}','E_LITERAL_KIND'],
 ['pub fnc main(): int {const age=42; val z={:age,age:1}; return 0;}','E_DUPLICATE'],
 ['pub fnc main(): int {const age=42; val z={:age,"x"=>1}; return 0;}','E_LITERAL_KIND'],
 ['pub fnc main(): int {let Record r={x:42}; let string key="x"; return r[key];}','E_UNSUPPORTED'],
 ['pub fnc main(): int {let Record r={x:42}; return r["missing"]; }','E_NO_FIELD'],
 ['pub fnc main(): int {let Map<string,int> m={"x"=>42}; return m["x"]; }','E_UNSUPPORTED'],
 ['pub fnc main(): int {let Record r={x:1,"x":2}; return 0; }','E_LITERAL_KIND'],
 ['pub fnc main(): int {const Record r={x:42}; return r.x;}','E_UNSUPPORTED'],
 ['pub fnc main(): int {let Record headers={content_type:42}; block {let headers=7; stdio.println(headers["content_type"]);} return 0;}','E_UNSUPPORTED'],
 ['define struct Header as int status_code; end pub fnc main(): int {val h=Header{status_code:42}; return h["status_code"]; }','E_UNSUPPORTED'],
 ['pub fnc main(): int {let Record headers={content_type:42}; return headers["content"+"-type"]; }','E_UNSUPPORTED'],
 ['pub fnc main(): int {let Record headers={café:42}; return headers["cafe_decomp"]; }','E_NO_FIELD'],
 ['pub fnc main(): int {let Record headers={content_type:42}; return headers[0]; }','E_UNSUPPORTED'],
];
// Additional full-contract dot/bracket tests including decoded string escapes.
positives.push(["fnc double(value: int): int { return value + value; } pub fnc main(): int { let Record headers = {content_type:\"application/json\",status_code:21}; let int a=headers.status_code; let int b=headers[\"status_code\"]; stdio.println(headers[\"content_type\"]); return double(a)+b-21; }", 42n, "application/json\n"]);
positives.push(["pub fnc main(): int { let Record h={quote_key:3,slash_key:4,tab_key:5,unicode:30}; return h[\"quote_key\"]+h[\"slash_key\"]+h[\"tab_key\"]+h[\"unicode\"]; }", 42n, ""]);
positives.push(["pub fnc main(): int { let Record h={status_code:21,content_type:21}; return h.status_code+h[\"content_type\"]; }", 42n, ""]);
negatives.push(["pub fnc main(): int {let Record h={content_type:\"json\",status_code:42}; let int bad=h[\"content_type\"];return bad;}", "E_TYPE"]);
negatives.push(["pub fnc main(): int {let Record h={content_type:\"json\",status_code:42}; let string bad=h.status_code;return 0;}", "E_TYPE"]);
negatives.push(["pub fnc main(): int {let Record h={content_type:21,content_type:21};return 0;}", "E_DUPLICATE"]);
negatives.push(["pub fnc main(): int {let Record h={content_type:\"json\",status_code:42};return h.missing;}", "E_NO_FIELD"]);
negatives.push(["pub fnc main(): int {let Record h={nested:{status_code:42}};return h.nested[\"status_code\"];}", "E_UNSUPPORTED"]);
negatives.push(["pub fnc main(): int {let Record h={\"\\u{D800}\":42};return 0;}", "E_STRING"]);
// Additional round-trip escapes, newline names and exact field-return typing.
positives.push([String.raw`pub fnc main(): int {let Record h={x_y:41,has_quote:1,line_break:2}; return h["x_y"]+h["has_quote"];} `, 42n, ""]);
positives.push([String.raw`pub fnc main(): int {let Record h={line_break:42}; return h["line_break"];} `, 42n, ""]);
negatives.push([String.raw`pub fnc main(): int {let Record h={x_y:1,x_y:2}; return 0;} `, "E_DUPLICATE"]);
negatives.push([String.raw`pub fnc main(): int {let Record h={x_y:42}; return h["x\u{zz}y"];} `, "E_STRING"]);
negatives.push([String.raw`pub fnc main(): int {let Record h={content_type:"json",status_code:42}; return h["content_type"];} `, "E_TYPE"]);
// Boundary/identity cases: each positive guest is executed, and every negative
// must be rejected by both actual compiler implementations with zero artifacts.
positives.push([
 'pub fnc main(): int {let Record r={h:35,g:2,f:3,e:4,d:5,c:6,b:8,a:7}; return r["a"]+r.h;}',
 42n, ''
]);
positives.push([
 'pub fnc main(): int {let Record one={value:42}; let Record two={value:true}; two.value; return one["value"]; }',
 42n, ''
]);
positives.push([
 'pub fnc main(): int {let Record r={f:1.5,b:true,s:"ok",i:42}; r["f"]; r["b"]; r["s"]; return r["i"]; }',
 42n, ''
]);
positives.push([
 'pub fnc main(): int {let Record r={launch:40,x_y_z:2}; return r["launch"]+r["x_y_z"]; }',
 42n, ''
]);
positives.push([
 'pub fnc main(): int {let Record r={empty:41,x_y:1}; val copy=r; return copy["empty"]+copy["x_y"]; }',
 42n, ''
]);
positives.push([
 'pub fnc main(): int {let Record one={n:41,label:"first"}; let Record two={label:"second",n:1}; return one.n+two["n"]; }',
 42n, ''
]);
negatives.push([
 'pub fnc main(): int {let Record r={a:1,b:2,c:3,d:4,e:5,f:6,g:7,h:8,i:9}; return r["a"]; }',
 'E_UNSUPPORTED'
]);
negatives.push([
 'pub fnc main(): int {let Record r={}; return 0; }',
 'E_UNSUPPORTED'
]);
negatives.push([
 'pub fnc main(): int {let Record r={launch:42}; return r["Launch"]; }',
 'E_NO_FIELD'
]);
negatives.push([
 'pub fnc main(): int {let Record r={x:42}; let int other=7; return other["x"]; }',
 'E_UNSUPPORTED'
]);
negatives.push([
 'pub fnc main(): int {let Record r={i:42,s:"ok"}; let int wrong=r["s"]; return wrong; }',
 'E_TYPE'
]);


// Checked owned static Record mutation: real native/browser-compiler-produced
// guest Wasm must write a fixed slot, not fabricate a Map, alias or interpreter.
positives.push([
 'pub fnc main(): int {let mut Record r={foo:1}; r["foo"]=42; return r.foo;}',
 42n, ''
]);
positives.push([
 'pub fnc main(): int {val mut Record r={content_type:1}; r["content_type"]=42; return r["content_type"]; }',
 42n, ''
]);
positives.push([
 'pub fnc main(): int {let mut Record r={foo:1,bar:5}; r.foo=37; return r["foo"]+r.bar; }',
 42n, ''
]);
positives.push([
 'pub fnc main(): int {let mut Record r={empty:1,café:2,cafe_decomp:3}; r["empty"]=37; return r["empty"]+r["café"]+r["cafe_decomp"]; }',
 42n, ''
]);
positives.push([
 'pub fnc main(): int {let mut Record r={flag:false,code:1}; r["flag"]=true; r["code"]=41; return r.code + (r.flag ? 1 : 0); }',
 42n, ''
]);
negatives.push([
 'pub fnc main(): int {let Record r={foo:1}; r["foo"]=42; return 0;}',
 'E_IMMUTABLE'
]);
negatives.push([
 'pub fnc main(): int {val Record r={foo:1}; r.foo=42; return 0;}',
 'E_IMMUTABLE'
]);
negatives.push([
 'pub fnc main(): int {let mut Record r={foo:1}; r["bar"]=42; return 0;}',
 'E_NO_FIELD'
]);
negatives.push([
 'pub fnc main(): int {let mut Record r={foo:1}; r["foo"]="bad"; return 0;}',
 'E_TYPE'
]);
negatives.push([
 'pub fnc main(): int {let mut Record r={foo:1}; let string key="foo"; r[key]=42; return 0;}',
 'E_UNSUPPORTED'
]);
negatives.push([
 'pub fnc main(): int {let mut Record r={foo:1}; r["f"+"oo"]=42; return 0;}',
 'E_UNSUPPORTED'
]);
negatives.push([
 'pub fnc main(): int {let mut Record r={foo:1}; val reader=borrow! r; r["foo"]=42; return 0;}',
 'E_BORROW'
]);
negatives.push([
 'pub fnc main(): int {let mut Record r={foo:1}; val moved=take! r; r["foo"]=42; return 0;}',
 'E_MOVED'
]);
negatives.push([
 'define struct Point as int foo; end pub fnc main(): int {let mut p=Point{foo:1}; p["foo"]=42; return 0;}',
 'E_UNSUPPORTED'
]);
negatives.push([
 'define struct Point as int foo; end pub fnc main(): int {val p=Point{foo:1}; return p["foo"]; }',
 'E_UNSUPPORTED'
]);

// Named parenthesized Records MUST lower through the same checked scalar
// shape ABI as braced Records, not a Tuple, Map, or unverified guest object.
positives.push([
 'pub fnc main(): int { let y=(foo="star",abc=5); return y.abc; }',
 5n,''
]);
positives.push([
 'pub fnc main(): int { let y=(abc=5,foo="star"); return y.abc; }',
 5n,''
]);
positives.push([
 'pub fnc main(): int { let y=(foo=42); return y.foo; }',
 42n,''
]);
positives.push([
 'pub fnc main(): int { let y=(foo=42,); return y["foo"]; }',
 42n,''
]);
positives.push([
 'pub fnc main(): int { let y=(foo=40,bar=2); return y.foo+y.bar; }',
 42n,''
]);
positives.push([
 'pub fnc main(): int { let y=(bar=2,foo=40,); return y["foo"]+y.bar; }',
 42n,''
]);
positives.push([
 'pub fnc main(): int { let mut r=(foo=1,bar=2); r["foo"]=40; return r.foo+r.bar; }',
 42n,''
]);
positives.push([
 'pub fnc main(): int { let r=(foo=1,bar=2); let z={bar:2,foo:1}; return r.foo+z.foo+40; }',
 42n,''
]);
positives.push([
 'pub fnc main(): int { val mut Record r=(foo=2); r.foo=42; return r["foo"]; }',
 42n,''
]);
negatives.push([
 'pub fnc main(): int { let y=(foo=1,foo=2); return 0; }', 'E_DUPLICATE'
]);
negatives.push([
 'pub fnc main(): int { let y=(foo=1,2); return 0; }', 'E_LITERAL_KIND'
]);
negatives.push([
 'pub fnc main(): int { let y=(foo=1,bar:2); return 0; }', 'E_LITERAL_KIND'
]);
negatives.push([
 'pub fnc main(): int { let y=(foo=1,"bar"=2); return 0; }', 'E_LITERAL_KIND'
]);
negatives.push([
 'pub fnc main(): int { let y=("foo"=1); return 0; }', 'E_LITERAL_KIND'
]);
negatives.push([
 'pub fnc main(): int { let y=(foo=1,,bar=2); return 0; }', 'E_LITERAL_KIND'
]);
negatives.push([
 'pub fnc main(): int { let y=(foo=(bar=1)); return 0; }', 'E_UNSUPPORTED'
]);
negatives.push([
 'pub fnc main(): int { let y=(foo=1,bar=[1,2]); return 0; }', 'E_UNSUPPORTED'
]);
negatives.push([
 'pub fnc main(): int { let y:Map<string,int>=(foo=1); return 0; }', 'E_TYPE'
]);
negatives.push([
 'pub fnc main(): int { let y=(foo=1); y["foo"]=42; return 0; }', 'E_IMMUTABLE'
]);
negatives.push([
 'pub fnc main(): int { let y=(foo=1); return y["missing"]; }', 'E_NO_FIELD'
]);
negatives.push([
 'pub fnc main(): int { let y={}; return 0; }', 'E_INFER'
]);
negatives.push([
 'pub fnc main(): int { let y=(); return 0; }', 'E_UNSUPPORTED'
]);
negatives.push([
 'fnc f(int foo):int {return foo;} pub fnc main(): int {return f(foo=1);}', 'E_UNSUPPORTED'
]);
negatives.push([
 'fnc f(int foo):int {return foo;} pub fnc main(): int {let r=(foo=1);return f(...r);}', 'E_UNSUPPORTED'
]);

// Exhaustive guest execution: four independent scalar fields, 24 source orders.
// Both compilers must produce byte-identical Wasm; every module must return 42.
function visit_record_orders(items, start, visit) {
 if (start === items.length) { visit(items); return; }
 for (let i=start; i<items.length; ++i) {
  [items[start],items[i]]=[items[i],items[start]];
  visit_record_orders(items,start+1,visit);
  [items[start],items[i]]=[items[i],items[start]];
 }
}
const orderedFields=[['a',10],['b',11],['c',12],['d',9]];
visit_record_orders([0,1,2,3],0,order=>{
 const parts=order.map(i=>orderedFields[i][0]+': '+orderedFields[i][1]);
 const source='pub fnc main(): int { let Record r={'+parts.join(',')+'}; return r.a+r["b"]+r.c+r["d"]; }';
 positives.push([source,42n,'']);
});
// Audit parity: source-field permutation cannot change a named Record's
// canonical shape, source value evaluation order, or emitted guest behavior.
visit_record_orders([0,1,2,3],0,order=>{
 const parts=order.map(i=>orderedFields[i][0]+'='+orderedFields[i][1]);
 const source='pub fnc main(): int { let r=('+parts.join(',')+'); return r.a+r.b+r.c+r.d; }';
 positives.push([source,42n,'']);
});
positives.push([
 'pub fnc main():int {let r=(Foo=40,foo=2);return r.Foo+r.foo;}',42n,''
]);
positives.push([
 'pub fnc main():int {let r=(café=40,cafe=2);return r.café+r.cafe;}',42n,''
]);
positives.push([
 'pub fnc main():int {let r=(f0=1,f1=1,f2=1,f3=1,f4=1,f5=1,f6=1,f7=35);return r.f0+r.f1+r.f2+r.f3+r.f4+r.f5+r.f6+r.f7;}',42n,''
]);
positives.push([
 'pub fnc main():int {return 3>=2 ? 42 : 0;}',42n,''
]);
for(const literal of [
 '(foo=1,foo=2)',
 '(café=1,café=2)',
 '{foo:1,"foo":2}',
 '{"foo":1,foo:2}',
 '{"foo" => 1,"foo" => 2}',
 '{1 => true,1 => false}',
]){
 const kind = literal.includes('foo:1,"foo"') || literal.includes('"foo":1,foo:')
     ? 'E_LITERAL_KIND' : 'E_DUPLICATE';
 negatives.push(['pub fnc main():int {let r='+literal+';return 0;}',kind]);
}
for(const literal of [
 '(foo=1,bar)',
 '(foo=1,bar==2)',
 '(foo=1,if=2)',
 '(foo=1,bar:2)',
 '(foo=1,"bar"=2)',
 '(foo=1,bar=2,3)',
]){
 negatives.push(['pub fnc main():int {let r='+literal+';return 0;}','E_LITERAL_KIND']);
}
negatives.push([
 'pub fnc main():int {let r=(__atto_closed_record_shape_0=42);return 0;}', 'E_RESERVED'
]);
negatives.push([
 'pub fnc main():int {let r=(f0=1,f1=1,f2=1,f3=1,f4=1,f5=1,f6=1,f7=1,f8=1);return 0;}', 'E_UNSUPPORTED'
]);
negatives.push([
 'pub fnc main():int {let r:Map<string,int>={};return 0;}', 'E_UNSUPPORTED'
]);
// Check the source-level 1,024-field bound on native AND independent Wasm
// compiler in an unprivileged process, without evaluating a partial guest.
const oversizedNamed=(new Array(1025)).fill(0).map((_,i)=>'field_'+i+'=1').join(',');
negatives.push([
 'pub fnc main():int {let r=('+oversizedNamed+');return 0;}', 'E_SOURCE_LIMIT'
]);
negatives.push([
 'pub fnc main():int {stdio.println(value=42);return 0;}', 'E_UNSUPPORTED'
]);

negatives.push(["pub fnc main():int {let m={\"foo\":5};return 0;}", "E_UNSUPPORTED"]);
negatives.push(["pub fnc main():int {let m={'foo':5};return 0;}", "E_UNSUPPORTED"]);
negatives.push(["pub fnc main():int {let m={`foo`:5};return 0;}", "E_UNSUPPORTED"]);
negatives.push(["pub fnc main():int {let m={\"foo\"=>5};return 0;}", "E_UNSUPPORTED"]);
negatives.push(["pub fnc main():int {let m={'foo'=>5};return 0;}", "E_UNSUPPORTED"]);
negatives.push(["pub fnc main():int {let m={`foo`=>5};return 0;}", "E_UNSUPPORTED"]);
negatives.push(["pub fnc main():int {let m={\"foo\":5,'foo'=>6};return 0;}", "E_DUPLICATE"]);
negatives.push(["pub fnc main():int {let m={foo:1,\"bar\":2};return 0;}", "E_LITERAL_KIND"]);
// Preserve safety boundaries without interpreting compile errors as guest tests.
negatives.push([
 'pub fnc main(): int {let Record r={x:42}; val clone=copy! r; return clone.x;}',
 'E_COPY'
]);
negatives.push([
 'pub fnc main(): int {let Record r={x:42}; val alias=share! r; return alias.x;}',
 'E_SHARE'
]);
negatives.push([
 'pub fnc main(): int {let Record r={x:42}; val moved=take! r; return r.x+moved.x;}',
 'E_MOVED'
]);
negatives.push([
 'define actor W as receive(ActorMail<int> mail): void {return;} end pub fnc main(): int {let Record r={x:42}; val w=spawn W(); w.send(r); return 0;}',
 'E_TYPE'
]);
negatives.push([
 'define actor W as receive(ActorMail<Record> mail): void {return;} end pub fnc main(): int {return 0;}',
 'E_UNSUPPORTED'
]);
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
