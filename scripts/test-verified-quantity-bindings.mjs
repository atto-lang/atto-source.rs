// Execute actual guest modules produced by attoc. No synthetic oracle substitutes.
import assert from "node:assert/strict";
import {spawnSync} from "node:child_process";
import {mkdtempSync,readFileSync,writeFileSync,existsSync,rmSync} from "node:fs";
import {tmpdir} from "node:os";
import {join,resolve} from "node:path";
const compiler=resolve(process.argv[2]);
const pre="define dimension Time; define dimension Length; define unit Second: Time; define unit Millis: Time = Second / 1000; define unit Meter: Length; type Duration = int where units in Millis; type Seconds = int where units in Second; type Distance = int where units in Meter;";
const ratio="fnc ratio(Duration a, Duration b) -> int { return a / b; } ";
const cases=[
 ["private-call",ratio+"pub fnc main() -> int { val Duration a = 84<Millis>; val b: Duration = 2<Millis>; return ratio(a,b); }",42n],
 ["typed-return","fnc twice(Duration a) -> Duration { return a + a; } pub fnc main() -> int { val Duration a = twice(21<Millis>); return a / 1<Millis>; }",42n],
 ["literal-conversion","pub fnc main() -> int { val Seconds a = 42000<Millis>.to_unit<Second>(); return a / 1<Second>; }",42n],
 ["negative-truncation",ratio+"pub fnc main() -> int { return ratio(-85<Millis>,2<Millis>); }",-42n],
 ["divide-zero",ratio+"pub fnc main() -> int { return ratio(1<Millis>,0<Millis>); }",2],
 ["divide-overflow",ratio+"pub fnc main() -> int { return ratio(-9223372036854775808<Millis>,-1<Millis>); }",1],
 ["add-overflow","fnc twice(Duration a) -> Duration { return a + a; } pub fnc main() -> int { return twice(9223372036854775807<Millis>) / 1<Millis>; }",1],
 ["bad-scale","pub fnc main() -> int { val Duration a = 1<Second>; return 42; }","E_UNIT_SCALE"],
 ["bad-dimension","pub fnc main() -> int { val Duration a = 1<Meter>; return 42; }","E_UNIT_DIMENSION"],
 ["bad-argument",ratio+"pub fnc main() -> int { return ratio(1<Second>,1<Millis>); }","E_UNIT_SCALE"],
 ["bad-return","pub fnc main() -> int { val Duration a = 1<Millis>; return a; }","E_UNIT_ERASURE"],
 ["unreachable-static-error",ratio+"pub fnc main() -> int { val int a = ratio(1<Millis>,0<Millis>); val Duration b = 1<Meter>; return a; }","E_UNIT_DIMENSION"],
];
const work=mkdtempSync(join(tmpdir(),"atto-typed-quantities-"));
try {
 for(const [id,body,expected] of cases){
  const input=join(work,id+".atto"),output=join(work,id+".wasm");
  writeFileSync(input,pre+"\n"+body);
  const r=spawnSync(compiler,[input,"-o",output],{cwd:work,encoding:"utf8",timeout:30000});
  if(r.error)throw r.error;
  assert.equal(r.signal,null,id);
  if(typeof expected==="string"){
   assert.equal(r.status,1,id);
   assert.match(r.stderr,new RegExp(": "+expected+": "),id);
   assert.equal(existsSync(output),false,id);
  }else{
   assert.equal(r.status,0,r.stderr);
   const m=new WebAssembly.Module(readFileSync(output));
   assert.deepEqual(WebAssembly.Module.imports(m),[],id);
   assert.deepEqual(WebAssembly.Module.exports(m).map(x=>x.name).sort(),["main","panic_code"],id);
   const i=new WebAssembly.Instance(m,{});
   if(typeof expected==="bigint"){assert.equal(i.exports.main(),expected,id);assert.equal(i.exports.panic_code.value,0,id);}
   else{assert.throws(()=>i.exports.main(),WebAssembly.RuntimeError,id);assert.equal(i.exports.panic_code.value,expected,id);}
  }
  console.log("PASS typed quantity "+id);
 }
 console.log("verified "+cases.length+" typed quantity source guests");
}finally{rmSync(work,{recursive:true,force:true});}
