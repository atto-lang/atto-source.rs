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
cases.push(["inferred-positive","pub fnc main() -> int { val a = 42.<Second>::<Positive>; return a / 1<Second>; }",42n],
 ["inferred-failure","pub fnc main() -> int { val a = 0.<Second>::<Positive>; return a / 1<Second>; }",7],
 ["inferred-unit-erasure","pub fnc main() -> int { val a = 42.<Second>; return a; }","E_UNIT_ERASURE"],
 ["inferred-parameter-proof","fnc f(Bounded x) -> int { return x; } pub fnc main() -> int { val a = (40 + 2); return f(a); }",42n],
 ["inferred-parameter-denial","fnc f(Bounded x) -> int { return x; } pub fnc main() -> int { val a = 43; return f(a); }","E_REFINEMENT_PRECONDITION"]);
// Strict profile checks all private helper bodies, not just main's result.
const strictCases = [
 ["literal-safe", main("40+2"),42n],
 ["alias-range", "fnc inc(Bounded x) -> int { return x + 1; } "+main("inc(41)"),42n],
 ["positive-denominator", "fnc divide(Positive x) -> int { return 42<Second> / x; } "+main("divide(1<Second>)"),42n],
 ["proven-cast", main("(42 as Bounded)"),42n],
 ["narrowed-call", "fnc narrow(Bounded x) -> Bounded { return x as Bounded; } "+main("narrow(42)"),42n],
 ["signed-min-valid", main("-9223372036854775808"),-9223372036854775808n],
 ["unknown-add", "fnc f(int x) -> int { return x + 1; } "+main("f(41)"),"E_VERIFY_UNPROVEN"],
 ["unknown-sub", "fnc f(int x) -> int { return x - 1; } "+main("f(43)"),"E_VERIFY_UNPROVEN"],
 ["unknown-mul", "fnc f(int x) -> int { return x * 2; } "+main("f(21)"),"E_VERIFY_UNPROVEN"],
 ["unknown-neg", "fnc f(int x) -> int { return -x; } "+main("f(-42)"),"E_VERIFY_UNPROVEN"],
 ["zero-denominator", main("42<Second> / 0<Second>"),"E_VERIFY_UNPROVEN"],
 ["min-div-neg-one", main("-9223372036854775808<Second> / -1<Second>"),"E_VERIFY_UNPROVEN"],
 ["unknown-denominator", "fnc f(Duration x) -> int { return 42<Second> / x; } "+main("f(1<Second>)"),"E_VERIFY_UNPROVEN"],
 ["unknown-cast", "fnc f(int x) -> Bounded { return x as Bounded; } "+main("f(42)"),"E_VERIFY_UNPROVEN"],
 ["eager-unused", "pub fnc main() -> int { val int x = 9223372036854775807 + 1; return 42; }","E_VERIFY_UNPROVEN"],
 ["eager-masked", main("(9223372036854775807+1)*0"),"E_VERIFY_UNPROVEN"],
 ["nested-cast-cannot-sanitize", main("(9223372036854775807+1) as Bounded"),"E_VERIFY_UNPROVEN"],
 ["partial-normal-range", "fnc f(int x) -> int { return (x + 1) as Bounded; } "+main("f(41)"),"E_VERIFY_UNPROVEN"],
 ["unreachable-private-helper", "fnc f(int x) -> int { return x + 1; } "+main("42"),"E_VERIFY_UNPROVEN"],
 ["bad-argument", "fnc f(Bounded x) -> int { return x; } "+main("f(43)"),"E_REFINEMENT_PRECONDITION"]
];
strictCases.push(
 ["guard-positive","fnc f(int x) -> int { if x > 0 { return 42 / x; } return 0; } "+main("f(1)"),42n],
 ["guard-false","fnc f(int x) -> int { if x > 0 { return 42 / x; } return 0; } "+main("f(0)"),0n],
 ["early-return","fnc f(int x) -> int { if x <= 0 { return 0; } return 42 / x; } "+main("f(1)"),42n],
 ["both-return","fnc f(int x) -> int { if x > 0 { return 42 / x; } else { return 7; } } "+main("f(0)"),7n],
 ["nested-min-guard","fnc f(int x) -> int { if x < 0 { if x > -9223372036854775808 { return -x; } } return 0; } "+main("f(-42)"),42n],
 ["quantity-guard","fnc f(Duration x) -> int { if x > 0<Second> { return 42<Second> / x; } return 0; } "+main("f(1<Second>)"),42n],
 ["branch-local","fnc f(int x) -> int { if x > 0 { val n = 42 / x; return n; } return 0; } "+main("f(1)"),42n],
 ["assignment-replaces","pub fnc main() -> int { let int x = 0; x = 1; return 42 / x; }",42n],
 ["join-safe","fnc f(int n) -> int { let int x = 0; if n > 0 { x = 1; } else { x = 2; } return 42 / x; } "+main("f(0)"),21n],
 ["join-safe-true","fnc f(int n) -> int { let int x = 0; if n > 0 { x = 1; } else { x = 2; } return 42 / x; } "+main("f(1)"),42n],
 ["assignment-stale","pub fnc main() -> int { let int x = 1; x = 0; return 42 / x; }","E_VERIFY_UNPROVEN"],
 ["branch-stale","fnc f(int n) -> int { let int x = n; if x > 0 { x = 0; return 42 / x; } return 0; } "+main("f(1)"),"E_VERIFY_UNPROVEN"],
 ["join-unsafe","fnc f(int n) -> int { let int x = 1; if n > 0 { x = 0; } return 42 / x; } "+main("f(0)"),"E_VERIFY_UNPROVEN"],
 ["join-loses-narrowing","fnc f(int n) -> int { if n > 0 { val v = 42 / n; } return 42 / n; } "+main("f(1)"),"E_VERIFY_UNPROVEN"],
 ["interior-hole","fnc f(int n) -> int { if n != 0 { return 42 / n; } return 0; } "+main("f(1)"),42n],
 ["negative-min-guard","fnc f(int n) -> int { if n < 0 { return -n; } return 0; } "+main("f(-42)"),"E_VERIFY_UNPROVEN"],
 ["scope-leak","fnc f(int n) -> int { if n > 0 { val x = 42; } return x; } "+main("f(1)"),"E_UNIT_NAME"],
 ["immutable-rebind","pub fnc main() -> int { val x = 1; x = 2; return x; }","E_VERIFY_UNSUPPORTED"],
 ["refined-rebind","pub fnc main() -> int { let Bounded x = 1; x = 0; return x; }","E_REFINEMENT_BINDING"],
 ["missing-return","fnc f(int n) -> int { if n > 0 { return 42; } } "+main("f(1)"),"E_VERIFY_RETURN"],
 ["condition-unit-mismatch","fnc f(Duration n) -> int { if n > 0 { return 42; } return 0; } "+main("f(1<Second>)"),"E_UNIT_DIMENSION"]
);
strictCases.push(
 ["hole-negative","fnc f(int n) -> int { if n != 0 { return 42 / n; } return 0; } "+main("f(-2)"),-21n],
 ["hole-zero","fnc f(int n) -> int { if n != 0 { return 42 / n; } return 0; } "+main("f(0)"),0n],
 ["hole-copy","fnc f(int n) -> int { if n != 0 { val d = n; return 42 / d; } return 0; } "+main("f(1)"),42n],
 ["hole-join","fnc f(int n) -> int { let int d = 1; if n > 0 { d = -1; } return 42 / d; } "+main("f(1)"),-42n],
 ["hole-join-lost","fnc f(int n) -> int { let int d = 1; if n > 0 { d = 0; } return 42 / d; } "+main("f(1)"),"E_VERIFY_UNPROVEN"],
 ["and-guard","fnc f(int n) -> int { if n != 0 && 42 / n > 0 { return 42 / n; } return 0; } "+main("f(1)"),42n],
 ["and-short-circuit","fnc f(int n) -> int { if n != 0 && 42 / n > 0 { return 42 / n; } return 0; } "+main("f(0)"),0n],
 ["or-short-circuit","fnc f(int n) -> int { if n == 0 || 42 / n > 0 { return 42; } return 0; } "+main("f(0)"),42n],
 ["or-false-edge","fnc f(int n) -> int { if n == 0 || n == -1 { return 0; } return -9223372036854775808 / n; } "+main("f(2)"),-4611686018427387904n],
 ["or-join","fnc f(int n) -> int { if n < 0 || n > 0 { return 42 / n; } return 0; } "+main("f(-1)"),-42n],
 ["and-two-holes","fnc f(int n) -> int { if n != 0 && n != -1 { return -9223372036854775808 / n; } return 0; } "+main("f(2)"),-4611686018427387904n],
 ["negated-comparison","fnc f(int n) -> int { if !(n == 0) { return 42 / n; } return 0; } "+main("f(1)"),42n],
 ["precedence","fnc f(int n) -> int { if n == 0 || n != -1 && 42 / n > 0 { return 42; } return 0; } "+main("f(0)"),42n],
 ["grouped","fnc f(int n) -> int { if (n != 0 && (n < -1 || n > 0)) { return -9223372036854775808 / n; } return 0; } "+main("f(2)"),-4611686018427387904n],
 ["min-overflow-remains","fnc f(int n) -> int { if n != 0 { return -9223372036854775808 / n; } return 0; } "+main("f(-1)"),"E_VERIFY_UNPROVEN"],
 ["wrong-and-edge","fnc f(int n) -> int { if n == 0 && 42 / n > 0 { return 1; } return 0; } "+main("f(0)"),"E_VERIFY_UNPROVEN"],
 ["wrong-or-edge","fnc f(int n) -> int { if n != 0 || 42 / n > 0 { return 1; } return 0; } "+main("f(0)"),"E_VERIFY_UNPROVEN"],
 ["reversed-guard","fnc f(int n) -> int { if 42 / n > 0 && n != 0 { return 1; } return 0; } "+main("f(0)"),"E_VERIFY_UNPROVEN"],
 ["noncontiguous-and","fnc f(int n) -> int { if n != 0 & & n > 0 { return 1; } return 0; } "+main("f(1)"),"E_SYNTAX"],
 ["no-proof-after-rebind","fnc f(int n) -> int { let int d = n; if d != 0 && d != -1 { d = -1; return -9223372036854775808 / d; } return 0; } "+main("f(2)"),"E_VERIFY_UNPROVEN"]
);
for (const [id,body,want] of strictCases) cases.push(["strict-"+id,body,want,true]);
const dir=mkdtempSync(join(tmpdir(),"atto-refinement-casts-"));
try{
 for(const [id,body,want,strict] of cases){
  const input=join(dir,id+".atto"),output=join(dir,id+".wasm");
  writeFileSync(input,pre+body);
  const r=spawnSync(compiler,[...(strict?["--verify=strict"]:[]),input,"-o",output],{cwd:dir,encoding:"utf8",timeout:30000});
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
 // Strict mode cannot silently fall back to the ordinary compiler.
 for (const [id,source] of [
   ["ordinary-source",main("42")],
   ["closed-quantity","define dimension Time; define unit Second: Time; "+main("42<Second> / 1<Second>")]
 ]) {
   const input=join(dir,id+".atto"),output=join(dir,id+".wasm");
   writeFileSync(input,source);
   const r=spawnSync(compiler,["--verify=strict",input,"-o",output],{cwd:dir,encoding:"utf8",timeout:30000});
   if(r.error)throw r.error;
   assert.equal(r.status,1,id+": "+r.stderr);
   assert.match(r.stderr,/: E_VERIFY_UNSUPPORTED: /,id);
   assert.equal(existsSync(output),false,id);
 }
 console.log("verified "+cases.length+" real refinement cast source cases plus 2 strict fallback negatives");
}finally{rmSync(dir,{recursive:true,force:true});}
