import assert from "node:assert/strict";
import {spawnSync} from "node:child_process";
import {mkdtempSync,readFileSync,writeFileSync,existsSync,rmSync} from "node:fs";
import {tmpdir} from "node:os";
import {join,resolve} from "node:path";
import {createHash} from "node:crypto";
const compiler=resolve(process.argv[2]);
const main=e=>"pub fnc main() -> int { return "+e+"; }";
const pre="define dimension Time; define unit Second: Time; define unit Millis: Time = Second / 1000;";
const typed="type Duration = int where units in Second;";
const cases=[];
const add=(id,src,want)=>cases.push([id,src,want]);
for(const [style,cast] of [["prefix",(x,t)=>"<"+t+">"+x],["postfix",(x,t)=>x+"::<"+t+">"],["keyword",(x,t)=>x+" as "+t]]){
 add(style+"-i64",main(cast("42","i64")),42n);
 add(style+"-int",main(cast("(40 + 2)","int")),42n);
 add(style+"-float","pub fnc main() -> int { val converted = "+cast("42","float")+"; if converted eq 42.0 then return 42; fi return 0; }",42n);
 add(style+"-minimum",main(cast("(-9223372036854775808)","i64")),-9223372036854775808n);
 add(style+"-invalid",main(cast("42","bool")),"E_UNSUPPORTED_CAST");
 add(style+"-void",main(cast("42","void")),"E_TYPE");
 add(style+"-narrow",main(cast("42.0","int")),"E_UNSUPPORTED_CAST");
 for(const [kind,extra] of [["closed",""],["typed",typed]]){
  add(style+"-"+kind+"-compose",pre+extra+main("("+cast("42","i64")+").<Second> / 1<Second>"),42n);
  add(style+"-"+kind+"-no-erasure",pre+extra+main(cast("(42<Second>)","i64")),"E_UNIT_ERASURE");
 }
}
for(const [id,e,want] of [
 ["dot","84.<Second> / 2.<Second>",42n],
 ["chain","42::<i64>.<Second> / 1<Second>",42n],
 ["dot-expr","(40 + 2).<Second> / 1<Second>",42n],
 ["dot-zero","1.<Second> / 0.<Second>",2],
 ["dot-overflow","-9223372036854775808.<Second> / -1.<Second>",1],
 ["dot-convert","42000.<Millis>.to_unit<Second>() / 1<Second>",42n],
 ["dot-retag","42<Second>.<Millis> / 1<Millis>","E_UNIT_ATTACHMENT"],
 ["dot-scale","(1<Second> / 1<Millis>).<Second> / 1<Second>","E_UNIT_ATTACHMENT"],
 ["dot-type","42.<i64>","E_UNIT_NAME"],
 ["prefix-unit","<Second>42","E_UNSUPPORTED_CAST"],
 ["legacy","42::<Second> / 1<Second>",42n],
])for(const [kind,extra] of [["closed",""],["typed",typed]])add(id+"-"+kind,pre+extra+main(e),want);
add("typed-call",pre+typed+"fnc attach_seconds(int n) -> Duration { return n::<i64>.<Second>; } "+main("attach_seconds(42) / 1<Second>"),42n);
add("comparison","pub fnc main() -> int { if 1 < 2 then return 42; fi return 0; }",42n);
add("ordinary-decimal","pub fnc main() -> int { if 21.5 eq 21.5 then return 42; fi return 0; }",42n);
add("ordinary-colon",main("42 : int"),42n);
add("postfix-precedence",main("40 + 2::<i64>"),42n);
add("prefix-precedence",main("<i64>40 + 2"),42n);
add("malformed-postfix",main("42::<i64"),"reject");
add("malformed-prefix",main("<i64 42"),"reject");
add("generic-call-not-reinterpreted",main("calculate::<int>(42)"),"reject");
add("ordinary-dot-unit-not-admitted",main("42.<Second>"),"reject");
const dir=mkdtempSync(join(tmpdir(),"atto-cast-ascription-"));
try{
 for(const [id,src,want] of cases){
  const input=join(dir,id+".atto"),output=join(dir,id+".wasm");
  writeFileSync(input,src);
  const r=spawnSync(compiler,[input,"-o",output],{encoding:"utf8",cwd:dir,timeout:30000});
  if(r.error)throw r.error;
  assert.equal(r.signal,null,id);
  if(typeof want==="string"){
   assert.equal(r.status,1,id+": "+r.stderr);
   if(want!=="reject")assert.match(r.stderr,new RegExp(": "+want+": "),id);
   assert.equal(existsSync(output),false,id);
  }else{
   assert.equal(r.status,0,id+": "+r.stderr);
   const m=new WebAssembly.Module(readFileSync(output));
   assert.deepEqual(WebAssembly.Module.imports(m),[],id);
   const guest=new WebAssembly.Instance(m,{});
   if(typeof want==="bigint"){assert.equal(guest.exports.main(),want,id);assert.equal(guest.exports.panic_code.value,0,id);}
   else{assert.throws(()=>guest.exports.main(),WebAssembly.RuntimeError,id);assert.equal(guest.exports.panic_code.value,want,id);}
  }
  console.log("PASS cast/ascription "+id);
 }
 console.log("verified "+cases.length+" actual cast/ascription source cases");
 console.log("compiler_sha256="+createHash("sha256").update(readFileSync(compiler)).digest("hex"));
}finally{rmSync(dir,{recursive:true,force:true});}
