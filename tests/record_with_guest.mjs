// Actual native-compiler -> Wasm guest execution (not a static type oracle).
import assert from "node:assert/strict";
import {mkdtemp,writeFile,readFile,stat,rm} from "node:fs/promises";
import {tmpdir} from "node:os";
import {join} from "node:path";
import {spawnSync} from "node:child_process";
import {fileURLToPath} from "node:url";

const attoc=process.env.ATTOC||fileURLToPath(new URL("../target/debug/attoc",import.meta.url));
const positive=[
  {name:"unchanged source",source:"fnc main(): int { val r={a:7,b:5}; val s=r with {a:10}; return r.a*100+s.a*10+s.b; }",want:805n},
  {name:"grow and retype",source:'fnc main(): int { val r={age:30}; val s=r with {age:"thirty",active:true}; if s.age eq "thirty" then return r.age+1; else return 0; fi }',want:31n},
  {name:"computed chained source",source:"fnc main(): int { val v=({a:1,b:2} with {a:4}) with {c:5}; return v.a*100+v.b*10+v.c; }",want:425n},
  {name:"canonical independent source field order",source:"fnc main(): int { val a={x:1,y:true}; val b={y:false,x:2}; return a.x+b.x; }",want:3n},
  {name:"empty override",source:"fnc main(): int { val a={n:42}; val b=a with {}; return a.n+b.n; }",want:84n},
  {name:"chained updates",source:"fnc main(): int { val r={a:3,b:4}; val s=r with {a:5} with {b:7}; return r.a*100+s.a*10+s.b; }",want:357n},
  {name:"Record annotation",source:"fnc main(): int { val r: Record={a:20}; val s: record=r with {b:22}; return r.a+s.b; }",want:42n},
  {name:"checked computed replacement",source:"fnc main(): int { val r={n:20}; val s=r with {n:r.n+22}; return r.n+s.n; }",want:62n},
  {name:"boolean and float preserved",source:"fnc main():int { val r={f:2.5,active:true}; val s=r with {active:false}; if r.active then return 1; else return 0; fi }",want:1n},
  {name:"quoted identifier label",source:'fnc main():int { val r={"answer":40}; val s=r with {answer:42}; return s.answer; }',want:42n}
];
const negative=[
  {name:"quoted nonidentifier",source:'fnc main(): int { val r={"content-type":"x"};return 0;}',want:"E_UNSUPPORTED"},
  {name:"nested graph",source:"fnc main(): int { val r={a:{nested:1}};return 0;}",want:"E_UNSUPPORTED"},
  {name:"unknown computed value",source:"fnc main(): int { val r={a:1}; val s=r with {b:unknown()};return 0;}",want:"E_WITH_TYPE"},
  {name:"open receiver",source:"fnc f(r:Record):int { val y=r with {a:1};return 0; } fnc main():int{return 0;}",want:"E_WITH_OPEN_RECORD"},
  {name:"incompatible result type",source:'fnc main():int { val x={a:1}; val y=x with {a:"str"}; val z:int=y.a;return 0;}',want:"E_TYPE"},
  {name:"duplicate override",source:"fnc main():int { val x={a:1}; val y=x with {a:2,a:3};return 0;}",want:"E_DUPLICATE"},
  {name:"map patch",source:"fnc main():int { val x={a:1}; val y=x with {a=>2};return 0;}",want:"E_WITH_MAP"},
  {name:"runtime constant object forbidden",source:"fnc main():int { const r={a:1};return 0;}",want:"E_CONST"},
  {name:"forged hidden shape name forbidden",source:"fnc main():int { val r=__atto_shape_with_record_0;return 0;}",want:"E_RESERVED"}
];
const work=await mkdtemp(join(tmpdir(),"atto-with-guest-"));
try {
  for(const [index,c] of positive.entries()){
    const input=join(work,"pass_"+index+".atto"),out=join(work,"pass_"+index+".wasm");
    await writeFile(input,c.source);
    const built=spawnSync(attoc,[input,"-o",out],{encoding:"utf8",timeout:30000});
    assert.equal(built.status,0,c.name+": compiler failed: "+built.stderr);
    const bytes=await readFile(out);
    assert.equal(WebAssembly.validate(bytes),true,c.name+": invalid Wasm");
    const module=new WebAssembly.Module(bytes);
    assert.deepEqual(WebAssembly.Module.imports(module),[],c.name+": unexpected imports");
    const guest=new WebAssembly.Instance(module,{});
    assert.equal(guest.exports.main(),c.want,c.name+": wrong guest result");
    assert.equal(guest.exports.panic_code.value,0,c.name+": unexpected panic");
    console.log("PASS Wasm guest",c.name,String(c.want));
  }
  // A failing update must trap before publishing the new root.
  // This currently exercises the compiler's documented fixed bump-heap ABI.
  const overflowSource="fnc main():int { val r={n:42}; val s=r with {n:r.n+9223372036854775807}; return r.n; }";
  const overflowFile=join(work,"overflow.atto"),overflowWasm=join(work,"overflow.wasm");
  await writeFile(overflowFile,overflowSource);
  const overflowBuilt=spawnSync(attoc,[overflowFile,"-o",overflowWasm],{encoding:"utf8",timeout:30000});
  assert.equal(overflowBuilt.status,0,"overflow negative must compile with runtime guard");
  const overflowModule=new WebAssembly.Module(await readFile(overflowWasm));
  const overflowGuest=new WebAssembly.Instance(overflowModule,{});
  assert.throws(()=>overflowGuest.exports.main(),WebAssembly.RuntimeError,"overflow must trap");
  assert.equal(overflowGuest.exports.panic_code.value,1,"overflow panic_code");
  assert.equal(new DataView(overflowGuest.exports.memory.buffer).getBigInt64(0,true),42n,
    "source Record must stay unchanged after failed update");
  console.log("PASS runtime overflow leaves source root intact");
  for(const [index,c] of negative.entries()){
    const input=join(work,"fail_"+index+".atto"),out=join(work,"fail_"+index+".wasm");
    await writeFile(input,c.source);
    const built=spawnSync(attoc,[input,"-o",out],{encoding:"utf8",timeout:30000});
    assert.equal(built.status,1,c.name+": unsafe source compiled: "+built.stderr);
    assert.ok(built.stderr.includes(c.want),c.name+": expected "+c.want+", got: "+built.stderr);
    await assert.rejects(stat(out),{code:"ENOENT"},c.name+": emitted unsafe Wasm");
    console.log("PASS expected rejection",c.name,c.want);
  }
}finally{
  await rm(work,{recursive:true,force:true});
}
