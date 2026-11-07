// Exact-source compiler ABI conformance for top-level define declarations.
// This tests the real emitted wasm32 compiler, and real emitted guest Wasm.
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
const binary=await readFile(process.argv[2]||'target/wasm32-unknown-unknown/release/atto_compiler.wasm');
assert.ok(WebAssembly.validate(binary));
const module=new WebAssembly.Module(binary);
assert.deepEqual(WebAssembly.Module.imports(module),[]);
const {exports:api}=new WebAssembly.Instance(module,{});
assert.equal(api.abi_version(),2);
const enc=new TextEncoder(),dec=new TextDecoder();
function compile(source) {
  const input=enc.encode(source);
  assert.ok(input.length<=api.input_capacity());
  new Uint8Array(api.memory.buffer,api.input_ptr(),input.length).set(input);
  const code=api.compile(input.length);
  if(code) {
    assert.equal(api.result_length(),0,'error must have no guest');
    const bytes=new Uint8Array(api.memory.buffer);
    const ptr=api.error_ptr();let end=ptr;
    while(end<ptr+512&&bytes[end])end++;
    return{ok:false,message:dec.decode(bytes.slice(ptr,end)),line:api.error_line_number()};
  }
  const guest=new Uint8Array(api.memory.buffer,api.result_ptr(),api.result_length()).slice();
  assert.ok(WebAssembly.validate(guest),'emitted guest must be valid Wasm');
  return{ok:true,guest};
}
function inspect(guest) {
  const mod=new WebAssembly.Module(guest);
  const imports=WebAssembly.Module.imports(mod);
  assert.deepEqual(imports,[], 'declaration-only guest must not acquire imports');
  const sections=WebAssembly.Module.customSections(mod,'atto.declarations.v1');
  assert.equal(sections.length,1, 'declaration metadata must not be silently erased');
  const bytes=new Uint8Array(sections[0]);let at=0;
  function uleb(){let n=0,shift=0,b;do{b=bytes[at++];n|=(b&127)<<shift;shift+=7;assert.ok(shift<=28);}while(b&128);return n;}
  function name(){const n=uleb(),v=dec.decode(bytes.slice(at,at+n));at+=n;return v;}
  assert.equal(uleb(),1,'declaration schema version');
  const count=uleb(),decls=[];
  for(let i=0;i<count;i++){
    const kind=bytes[at++],nameValue=name(),n=uleb(),members=[];
    for(let j=0;j<n;j++){
      const tag=bytes[at++],memberName=name();
      if(tag===1) {
        const type=bytes[at++],isFinal=bytes[at++],initTag=bytes[at++];
        let initial=null;
        if(initTag===1){initial=new DataView(bytes.buffer,bytes.byteOffset+at,8).getBigInt64(0,true);at+=8;}
        if(initTag===2)initial=bytes[at++]===1;
        members.push({name:memberName,tag,type,isFinal,initial});
      }else members.push({name:memberName,tag});
    }
    decls.push({kind,name:nameValue,members});
  }
  assert.equal(at,bytes.length,'no trailing unparsed metadata');
  return decls;
}
const source=[
  'define class Counter as',
  ' final int initial: 5',
  ' bool ready: true',
  'end',
  'define actor Worker as',
  ' final int id: 9',
  'end',
  'define enum Status as',
  ' #ready,',
  ' #done',
  'end',
  'fnc main() -> int {return 42;}',
].join('\n');
const result=compile(source);
assert.ok(result.ok,JSON.stringify(result));
const repeated=compile(source);
assert.deepEqual(repeated.guest,result.guest,'deterministic declared Wasm output');
const decls=inspect(result.guest);
assert.deepEqual(decls.map(d=>[d.kind,d.name]),[[1,'Counter'],[2,'Worker'],[3,'Status']]);
assert.equal(decls[0].members[0].initial,5n);
assert.equal(decls[0].members[0].isFinal,1);
assert.equal(decls[0].members[1].initial,true);
assert.equal(decls[2].members[0].tag,3);
assert.equal(decls[2].members[1].tag,3);
const {exports:guest}=new WebAssembly.Instance(new WebAssembly.Module(result.guest),{});
assert.equal(guest.main(),42n);
const only=compile('define pub struct Point {int x: 3\n int y: 4}\ndefine interface Shape as end\ndefine trait Renderable as end\n');
assert.ok(only.ok,JSON.stringify(only));
assert.equal(new WebAssembly.Instance(new WebAssembly.Module(only.guest),{}).exports.main(),0n);
assert.equal(inspect(only.guest).length,3);
const fn=compile('define pub fnc main() -> int {return twice(21);}\ndefine fnc twice(int x): int {return x + x;}\n');
assert.ok(fn.ok,JSON.stringify(fn));
assert.equal(new WebAssembly.Instance(new WebAssembly.Module(fn.guest),{}).exports.main(),42n);
assert.equal(WebAssembly.Module.customSections(new WebAssembly.Module(fn.guest),'atto.declarations.v1').length,0);
const invalid=[
 ['E_DUPLICATE','define actor Box as end\ndefine class Box as end'],
 ['E_DUPLICATE','define enum Status as Ready, Ready end'],
 ['E_DUPLICATE','define class Box as int x: 1\n int x: 2\nend'],
 ['E_EOF','define class Box as int x: 2\n'],
 ['E_UNSUPPORTED','define actor Worker as receive(msg: int) {return;} end'],
 ['E_UNSUPPORTED','define class Box extends Parent as end'],
 ['E_UNSUPPORTED','define class Box as int x: 2 + 2\nend'],
 ['E_TYPE','define class Box as int x: true\nend'],
 ['E_UNSUPPORTED','define class Box as end\nfnc main() -> int {let x = new Box();return 0;}'],
 ['E_UNSUPPORTED','define actor Worker as end\nfnc main() -> int {spawn Worker();return 0;}'],
 ['E_UNSUPPORTED','fnc main() -> int {return #ready;}'],
 ['E_UNSUPPORTED','define module Example as end'],
];
for(const [code,source] of invalid){
  const r=compile(source);
  assert.equal(r.ok,false,'must reject unsupported/malformed source: '+source);
  assert.ok(r.message.startsWith(code+':'),r.message);
  assert.ok(r.line>=1);
}
console.log('PASS top-level declarations: class actor enum struct interface trait define fnc, metadata, guests, deterministic and 12 negative gates');
