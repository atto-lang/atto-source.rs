import test from 'node:test';
import assert from 'node:assert/strict';
import {
  ACTOR_GUEST_IMPORTS, validate_actor_guest_imports,
} from './browser/actor-import-contract.mjs';

// Construct minimal *real* Wasm modules instead of mocking Module.imports().
// This exercises the browser's actual WebAssembly import descriptor decoder.
const stringBytes=s=>[...new TextEncoder().encode(s)];
const leb=n=>{
  if(!Number.isInteger(n)||n<0)throw new Error('invalid count');
  const out=[];
  do { const b=n&0x7f; n=Math.floor(n/128);out.push(n?b|0x80:b) } while(n);
  return out;
};
const name=s=>[...leb(stringBytes(s).length),...stringBytes(s)];
const section=(id,data)=>[id,...leb(data.length),...data];
function moduleWithImports(descriptors){
  const typeSection=section(1,[1,0x60,0,0]); // () -> () function type 0
  const imports=[];
  for(const {module='atto',field='actor_send',kind='function'} of descriptors){
    imports.push(...name(module),...name(field));
    switch(kind){
      case 'function': imports.push(0,0);break;  // kind+type index
      case 'table': imports.push(1,0x70,0,1);break; // externref? funcref, min=1
      case 'memory': imports.push(2,0,1);break; // min=1
      case 'global': imports.push(3,0x7f,0);break; // immutable i32
      default: throw new Error('test fixture kind');
    }
  }
  const bytes=new Uint8Array([
    0,97,115,109,1,0,0,0,
    ...typeSection,
    ...section(2,[...leb(descriptors.length),...imports]),
  ]);
  assert.equal(WebAssembly.validate(bytes),true,
    'test fixture must be a valid real Wasm binary');
  return new WebAssembly.Module(bytes);
}

test('empty Wasm guest imports are permitted',()=>{
  assert.deepEqual(validate_actor_guest_imports(moduleWithImports([])),[]);
});

test('each named Atto guest capability is granted once as a function',()=>{
  for(const field of ACTOR_GUEST_IMPORTS){
    const module=moduleWithImports([{field}]);
    assert.deepEqual(
      validate_actor_guest_imports(module),
      [{module:'atto',name:field,kind:'function'}],
    );
  }
});

test('all seven distinct imports can coexist without duplicates',()=>{
  const declarations=ACTOR_GUEST_IMPORTS.map(field=>({field}));
  const imports=validate_actor_guest_imports(moduleWithImports(declarations));
  assert.equal(imports.length,ACTOR_GUEST_IMPORTS.length);
  assert.deepEqual(imports.map(x=>x.name),ACTOR_GUEST_IMPORTS);
});

test('unknown module, member, Unicode lookalikes and near misses are rejected',()=>{
  const denied=[
    [{module:'env',field:'actor_send'}],
    [{module:'attox',field:'actor_send'}],
    [{module:'atto',field:'ActorSend'}],
    [{module:'atto',field:'actor_send_async'}],
    [{module:'atto',field:'actor_send\u200b'}],
    [{module:'atto',field:'actor\u005fsend\u0000'}],
    [{module:'atto',field:''}],
    [{module:'',field:'actor_send'}],
    [{module:'atto',field:'actor_shutdown'}],
  ];
  for(const declarations of denied){
    assert.throws(
      ()=>validate_actor_guest_imports(moduleWithImports(declarations)),
      /E_ACTOR_IMPORT_NAME/,JSON.stringify(declarations));
  }
});

test('memory/global/table imports cannot hide behind an allowed function name',()=>{
  for(const kind of ['global','memory','table']){
    for(const field of ['actor_send','actor_join','print_i64']){
      const guest=moduleWithImports([{field,kind}]);
      assert.throws(()=>validate_actor_guest_imports(guest),
        /E_ACTOR_IMPORT_KIND/,kind+':'+field);
    }
  }
});

test('duplicate import slots cannot alias one capability more than once',()=>{
  for(const field of ACTOR_GUEST_IMPORTS){
    const guest=moduleWithImports([{field},{field}]);
    assert.throws(()=>validate_actor_guest_imports(guest),
      /E_ACTOR_IMPORT_DUPLICATE/,field);
  }
});

test('more imports than the permitted capability set fail before member analysis',()=>{
  const declarations=ACTOR_GUEST_IMPORTS.map(field=>({field}));
  declarations.push({field:'actor_join'});
  assert.throws(
    ()=>validate_actor_guest_imports(moduleWithImports(declarations)),
    /E_ACTOR_IMPORT_COUNT/);
});

test('one valid import cannot disguise a second global with the same name',()=>{
  const guest=moduleWithImports([
    {field:'actor_send'},
    {field:'actor_send',kind:'global'},
  ]);
  assert.throws(()=>validate_actor_guest_imports(guest),
    /E_ACTOR_IMPORT_KIND/);
});

test('invalid bytes must be rejected by WebAssembly before import validation',()=>{
  const data=new Uint8Array([0,97,115,109,1,0,0,0,255]);
  assert.equal(WebAssembly.validate(data),false);
  assert.throws(()=>new WebAssembly.Module(data),WebAssembly.CompileError);
});
