import assert from "node:assert/strict";
import {spawnSync} from "node:child_process";
import {mkdtempSync,readFileSync,writeFileSync,existsSync,rmSync} from "node:fs";
import {tmpdir} from "node:os";
import {join,resolve} from "node:path";
import {createHash} from "node:crypto";
const compiler=resolve(process.argv[2]);
const pre="define dimension Time; define unit Second: Time; define unit Millis: Time = Second / 1000;";
const main=e=>"pub fnc main() -> int { return "+e+"; }";
const typed=b=>"type Duration = int where units in Second; "+b;
const cases=[];
for(const [style,tag] of [["compact",n=>n+"<Second>"],["symbolic",n=>n+"::<Second>"],["keyword",n=>"("+n+" as Second)"]]){
 for(const [id,a,b,want] of [["normal","84","2",42n],["zero","1","0",2],["overflow","-9223372036854775808","-1",1]]){
  cases.push([style+"-"+id,main(tag(a)+" / "+tag(b)),want]);
  cases.push(["typed-"+style+"-"+id,typed(main(tag(a)+" / "+tag(b))),want]);
 }
}
for(const [id,e,want] of [
 ["expression","(40 + 2)::<Second> / 1<Second>",42n],
 ["as-precedence","(40 + 2 as Second) / 1<Second>",42n],
 ["convert","42000::<Millis>.to_unit<Second>() / 1<Second>",42n],
 ["retag","1000<Millis>::<Second> / 1<Second>","E_UNIT_ATTACHMENT"],
 ["as-retag","(1000<Millis> as Second) / 1<Second>","E_UNIT_ATTACHMENT"],
 ["residual-scale","(1<Second> / 1<Millis>)::<Second> / 1<Second>","E_UNIT_ATTACHMENT"],
 ["double-tag","1::<Second>::<Second> / 1<Second>","E_UNIT_ATTACHMENT"],
 ["wrong-kind","1::<Time>","E_UNIT_NAME"],
 ["unknown","(1 as Unknown)","E_UNIT_NAME"],
]){cases.push([id,main(e),want]);cases.push(["typed-"+id,typed(main(e)),want]);}
for(const [id,body,want] of [
 ["parameter","fnc tag(int n) -> Duration { return n::<Second>; } "+main("tag(42) / 1<Second>"),42n],
 ["parameter-as","fnc tag(int n) -> Duration { return n as Second; } "+main("tag(42) / 1<Second>"),42n],
 ["local","pub fnc main() -> int { val int n = 42; val Duration a = n::<Second>; return a / 1<Second>; }",42n],
 ["parameter-reject","fnc tag(Duration n) -> Duration { return n as Second; } "+main("42"),"E_UNIT_ATTACHMENT"],
 ["attachment-overflow",main("(9223372036854775807 + 1)::<Second> / 1<Second>"),1],
])cases.push([id,typed(body),want]);
const work=mkdtempSync(join(tmpdir(),"atto-unit-attachment-"));
try{
 for(const [id,body,want] of cases){
  const input=join(work,id+".atto"),output=join(work,id+".wasm");
  writeFileSync(input,pre+"\n"+body);
  const r=spawnSync(compiler,[input,"-o",output],{cwd:work,encoding:"utf8",timeout:30000});
  if(r.error)throw r.error;
  assert.equal(r.signal,null,id);
  if(typeof want==="string"){
   assert.equal(r.status,1,id);
   assert.match(r.stderr,new RegExp(": "+want+": "),id);
   assert.equal(existsSync(output),false,id);
  }else{
   assert.equal(r.status,0,id+": "+r.stderr);
   const bytes=readFileSync(output),m=new WebAssembly.Module(bytes);
   assert.deepEqual(WebAssembly.Module.imports(m),[],id);
   assert.deepEqual(WebAssembly.Module.exports(m).map(x=>x.name).sort(),["main","panic_code"],id);
   const i=new WebAssembly.Instance(m,{});
   if(typeof want==="bigint"){assert.equal(i.exports.main(),want,id);assert.equal(i.exports.panic_code.value,0,id);}
   else{assert.throws(()=>i.exports.main(),WebAssembly.RuntimeError,id);assert.equal(i.exports.panic_code.value,want,id);}
  }
  console.log("PASS attachment "+id);
 }
 console.log("verified "+cases.length+" real unit attachment source cases");
 console.log("compiler_sha256="+createHash("sha256").update(readFileSync(compiler)).digest("hex"));
}finally{rmSync(work,{recursive:true,force:true});}
