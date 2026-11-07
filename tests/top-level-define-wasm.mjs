import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
const bytes=await readFile('target/wasm32-unknown-unknown/release/atto_compiler.wasm');
const mod=await WebAssembly.compile(bytes);
assert.deepEqual(WebAssembly.Module.imports(mod),[],'compiler itself cannot import host capabilities');
const encoder=new TextEncoder(),decoder=new TextDecoder();
function compile(source) {
  const w=new WebAssembly.Instance(mod,{}).exports;
  assert.equal(w.abi_version(),2);
  const input=encoder.encode(source);
  assert.ok(input.length<=w.input_capacity());
  new Uint8Array(w.memory.buffer,w.input_ptr(),input.length).set(input);
  const status=w.compile(input.length);
  if(status) {
    const mem=new Uint8Array(w.memory.buffer),start=w.error_ptr();
    let end=start;while(mem[end]&&end<start+512)end++;
    return {ok:false,diagnostic:decoder.decode(mem.subarray(start,end)),artifactBytes:w.result_length()};
  }
  const guest=new Uint8Array(w.memory.buffer,w.result_ptr(),w.result_length()).slice();
  assert.equal(WebAssembly.validate(guest),true);
  return {ok:true,guest};
}
async function run(source) {
  const result=compile(source);
  assert.equal(result.ok,true,result.diagnostic);
  const mod=await WebAssembly.compile(result.guest);
  assert.deepEqual(WebAssembly.Module.imports(mod),[],'no runtime imports for pure scalar declarations');
  const w=await WebAssembly.instantiate(mod,{});
  assert.equal(w.exports.main(),42n);
}
await run('define class Calc as\n static fnc answer(): int { return 42; }\nend\npub fnc main(): int { return Calc.answer(); }\n');
await run('define actor Worker as\n static fnc answer(): int { return 42; }\nend\npub fnc main(): int { return Worker.answer(); }\n');
await run('define enum Status as Pending, Ready end\nfnc is_ready(s: Status): bool { return s eq Status.Ready; }\npub fnc main(): int { if is_ready(Status.Ready) then return 42; fi return 0; }\n');
await run('define module Tools as\n fnc answer(): int { return 42; }\nend\npub fnc main(): int { return Tools.answer(); }\n');
const invalids=[
 ['define actor Worker as receive(x:int) { return 0; } end\npub fnc main(): int { return 0; }','E_UNSUPPORTED'],
 ['define enum A as Red, Red end\npub fnc main(): int { return 0; }','E_DUPLICATE'],
 ['define enum A as Red end\npub fnc main(): int { return A.Red + 1; }','E_TYPE'],
 ['define class X as\n fnc instance(): int {return 1;}\nend\npub fnc main(): int {return 0;}','E_UNSUPPORTED'],
];
for (const [source,code] of invalids) {
 const a=compile(source);
 assert.equal(a.ok,false,source);
 assert.equal(a.artifactBytes,0,source);
 assert.ok(a.diagnostic.startsWith(code+':'),a.diagnostic);
}
console.log('PASS real compiler wasm: class, actor, enum and module define, guest execution and fail-closed negative controls');
