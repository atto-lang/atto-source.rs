import assert from "node:assert/strict";
import {readFileSync,mkdtempSync,writeFileSync,rmSync} from "node:fs";
import {tmpdir} from "node:os";
import {join,resolve} from "node:path";
import {spawnSync} from "node:child_process";
import {createHash} from "node:crypto";
const [wasm,cli]=process.argv.slice(2);
assert.ok(wasm&&cli,"usage: node scripts/test-strict-browser-abi.mjs compiler.wasm attoc");
const bytes=readFileSync(wasm), module=new WebAssembly.Module(bytes);
assert.deepEqual(WebAssembly.Module.imports(module),[]);
const e=new WebAssembly.Instance(module,{}).exports;
assert.equal(typeof e.compile_strict_i64,"function");
assert.equal(typeof e.compile,"function");
assert.ok(e.memory instanceof WebAssembly.Memory);
const pre="define dimension Time; define unit Second: Time; type Bounded = int where self >= 1 and self <= 42; ";
const good=pre+"pub fnc main() -> int { return 42; }";
const unsafe=pre+"fnc f(int x) -> int { return x + 1; } pub fnc main() -> int { return f(41); }";
const lazy=pre+"fnc good(int x) -> int { val y=x; return y; } fnc f(int x) -> int { if x != 0 && good(42/x)>0 { return 1; } return 42; } pub fnc main() -> int { return f(0); }";
const loop=pre+"pub fnc main() -> int { let Bounded x = 1; while x < 42 { x = x + 1; } return x; }";
const encode=new TextEncoder(), decode=new TextDecoder("utf-8",{fatal:true});
const memory=()=>new Uint8Array(e.memory.buffer);
function call(source,strict=true,length){
 const input=typeof source==="string"?encode.encode(source):source;
 assert.ok(input.length<=e.input_capacity());
 memory().set(input,e.input_ptr());
 const status=(strict?e.compile_strict_i64:e.compile)(length??input.length);
 const n=e.result_length(), p=e.result_ptr(), err=e.error_ptr();
 assert.ok(err+512<=memory().length);
 const data=memory().slice(err,err+512), end=data.indexOf(0);
 assert.ok(end>=0,"unterminated diagnostic");
 const message=decode.decode(data.slice(0,end));
 if(status===0){
  assert.ok(n>8&&p+n<=memory().length);
  assert.equal(message,"");assert.equal(e.error_line_number(),0);assert.equal(e.error_column_number(),0);
  return {status,artifact:memory().slice(p,p+n)};
 }
 assert.equal(status,1);assert.equal(n,0,"stale artifact");assert.ok(e.error_line_number()>0&&e.error_column_number()>0);
 return {status,message};
}
const dir=mkdtempSync(join(tmpdir(),"strict-abi-"));
try {
 for(const [id,source] of [["scalar",good],["lazy",lazy],["loop",loop]]){
  const actual=call(source);assert.equal(actual.status,0,id);
  const src=join(dir,id+".atto"),out=join(dir,id+".wasm");writeFileSync(src,source);
  const r=spawnSync(resolve(cli),["--verify=strict",src,"-o",out],{encoding:"utf8",timeout:30000});
  if(r.error)throw r.error;assert.equal(r.status,0,r.stderr);
  assert.deepEqual(Buffer.from(actual.artifact),readFileSync(out),id+" CLI/Wasm bytes differ");
  const guest=new WebAssembly.Instance(new WebAssembly.Module(actual.artifact),{});
  assert.equal(guest.exports.main(),42n);assert.equal(guest.exports.panic_code.value,0);
  console.log("STRICT_ABI_IDENTICAL",id,createHash("sha256").update(actual.artifact).digest("hex"));
 }
 const denied=call(unsafe);assert.match(denied.message,/E_VERIFY_UNPROVEN:/);
 const ordinary=call(unsafe,false);assert.equal(ordinary.status,0);
 assert.equal(new WebAssembly.Instance(new WebAssembly.Module(ordinary.artifact),{}).exports.main(),42n);
 assert.match(call(unsafe).message,/E_VERIFY_UNPROVEN:/);
 for(const length of [-1,e.input_capacity()+1,2147483647]){
  assert.match(call("",true,length).message,/E_SOURCE_LIMIT:/);
  assert.equal(call(good).status,0);
 }
 assert.match(call(Uint8Array.of(0xff)).message,/E_UTF8:/);
 assert.equal(call(good).status,0);
 assert.match(call(unsafe).message,/E_VERIFY_UNPROVEN:/);
 assert.equal(call(good,false).status,0);
 console.log("STRICT_ABI_STATE_AND_NO_FALLBACK_PASS");
} finally {rmSync(dir,{recursive:true,force:true});}
