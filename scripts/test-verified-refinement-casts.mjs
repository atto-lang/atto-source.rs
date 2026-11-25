import assert from "node:assert/strict";
import {spawnSync} from "node:child_process";
import {mkdtempSync,readFileSync,writeFileSync,existsSync,rmSync} from "node:fs";
import {tmpdir} from "node:os";
import {join,resolve} from "node:path";
const compiler=resolve(process.argv[2]);
const pre="define dimension Time; define unit Second: Time; define unit Millis: Time = Second / 1000; type Duration = int where units in Second; type Positive = int where self > 0 and units in Second; type Bounded = int where self >= 1 and self <= 42; ";
const main=e=>"pub fnc main() -> int { return "+e+"; }";
const cases=[];
for(const [style,cast] of [["prefix",(x,t)=>"<"+t+">("+x+")"],["postfix",(x,t)=>"("+x+")::<"+t+">"],["keyword",(x,t)=>"("+x+" as "+t+")"]]){
 cases.push([style+"-positive",main(cast("42.<Second>","Positive")+" / 1<Second>"),42n]);
 cases.push([style+"-negative",main(cast("0.<Second>","Positive")+" / 1<Second>"),7]);
 cases.push([style+"-bounded",main(cast("42","Bounded")),42n]);
 cases.push([style+"-upper",main(cast("43","Bounded")),7]);
 cases.push([style+"-operand-trap",main(cast("1<Second> / 0<Second>","Bounded")),2]);
 cases.push([style+"-dimension",main(cast("42","Positive")),"E_UNIT_DIMENSION"]);
 cases.push([style+"-scale",main(cast("42<Millis>","Positive")),"E_UNIT_SCALE"]);
 cases.push([style+"-dynamic","fnc narrowed(int x) -> Bounded { return "+cast("x","Bounded")+"; } "+main("narrowed(42)"),42n]);
 cases.push([style+"-dynamic-fail","fnc narrowed(int x) -> Bounded { return "+cast("x","Bounded")+"; } "+main("narrowed(43)"),7]);
}
cases.push(["implicit-binding","pub fnc main() -> int { val Bounded n = 43; return n; }","E_REFINEMENT_BINDING"],
 ["implicit-return","fnc f(int x) -> Bounded { return x; } "+main("42"),"E_REFINEMENT_RETURN"],
 ["implicit-arg","fnc f(Bounded x) -> int { return x; } "+main("f(43)"),"E_REFINEMENT_PRECONDITION"],
 ["boundary-proven","fnc f(Bounded x) -> int { return x; } "+main("f(40+2)"),42n],
 ["contradiction","type Impossible = int where self > 0 and self < 1; "+main("42"),"E_REFINEMENT_EMPTY"],
 ["lower-extreme","type Min = int where self < -9223372036854775808; "+main("42"),"E_REFINEMENT_EMPTY"],
 ["upper-extreme","type Max = int where self > 9223372036854775807; "+main("42"),"E_REFINEMENT_EMPTY"],
 ["parameter-attachment","fnc tag(int n) -> Duration { return n.<Second>; } "+main("tag(42) / 1<Second>"),42n],
 ["parameter-retag","fnc tag(Duration n) -> Duration { return n.<Second>; } "+main("42"),"E_UNIT_ATTACHMENT"],
 ["cast-before-overflow",main("(43 as Bounded) + 9223372036854775807"),7],
 ["overflow-before-cast",main("(9223372036854775807 + 1) as Bounded"),1]);
const dir=mkdtempSync(join(tmpdir(),"atto-refinement-casts-"));
try{
 for(const [id,body,want] of cases){
  const input=join(dir,id+".atto"),output=join(dir,id+".wasm");
  writeFileSync(input,pre+body);
  const r=spawnSync(compiler,[input,"-o",output],{cwd:dir,encoding:"utf8",timeout:30000});
  if(r.error)throw r.error;
  assert.equal(r.signal,null,id);
  if(typeof want==="string"){
   assert.equal(r.status,1,id+": "+r.stderr);
   assert.match(r.stderr,new RegExp(": "+want+": "),id);
   assert.equal(existsSync(output),false,id);
  }else{
   assert.equal(r.status,0,id+": "+r.stderr);
   const m=new WebAssembly.Module(readFileSync(output));
   assert.deepEqual(WebAssembly.Module.imports(m),[],id);
   assert.deepEqual(WebAssembly.Module.exports(m).map(x=>x.name).sort(),["main","panic_code"],id);
   const instance=new WebAssembly.Instance(m,{});
   if(typeof want==="bigint"){
    assert.equal(instance.exports.main(),want,id);assert.equal(instance.exports.panic_code.value,0,id);
   }else{
    assert.throws(()=>instance.exports.main(),WebAssembly.RuntimeError,id);
    assert.equal(instance.exports.panic_code.value,want,id);
   }
  }
  console.log("PASS refinement "+id);
 }
 console.log("verified "+cases.length+" real refinement cast source cases");
}finally{rmSync(dir,{recursive:true,force:true});}
