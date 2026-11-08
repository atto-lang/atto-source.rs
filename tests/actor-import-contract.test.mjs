import test from 'node:test';
import assert from 'node:assert/strict';
import {
  ACTOR_GUEST_IMPORTS, ACTOR_GUEST_IMPORT_SIGNATURES,
  validate_actor_guest_imports,
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
function moduleWithImports(descriptors,{customBeforeImport=false}={}){
  const typeEntries=[],imports=[];
  for(let i=0;i<descriptors.length;i++){
    const {module='atto',field='actor_send',kind='function',params,results}=descriptors[i];
    const expected=ACTOR_GUEST_IMPORT_SIGNATURES[field] ||
      ACTOR_GUEST_IMPORT_SIGNATURES.actor_send;
    const ps=params??expected.params, rs=results??expected.results;
    typeEntries.push(0x60,...leb(ps.length),...ps,...leb(rs.length),...rs);
    imports.push(...name(module),...name(field));
    switch(kind){
      case 'function': imports.push(0,...leb(i));break;
      case 'table': imports.push(1,0x70,0,1);break;
      case 'memory': imports.push(2,0,1);break;
      case 'global': imports.push(3,0x7f,0);break;
      default: throw new Error('test fixture kind');
    }
  }
  const bytes=new Uint8Array([
    0,97,115,109,1,0,0,0,
    ...section(1,[...leb(descriptors.length),...typeEntries]),
    ...(customBeforeImport ? section(0,[...name('security.probe'),1,2,3]) : []),
    ...section(2,[...leb(descriptors.length),...imports]),
  ]);
  assert.equal(WebAssembly.validate(bytes),true,
    'test fixture must be a valid real Wasm binary');
  return bytes;
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

test('all nine distinct imports can coexist without duplicates',()=>{
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


test('exact Wasm function signatures are enforced for all nine capabilities',()=>{
  for(const [field,declared] of Object.entries(ACTOR_GUEST_IMPORT_SIGNATURES)){
    assert.deepEqual(
      validate_actor_guest_imports(moduleWithImports([{field}])),
      [{module:'atto',name:field,kind:'function'}],field);
    const wrongParam = declared.params.length===0 ? [0x7f] :
      declared.params.map((v,i)=>i===0?(v===0x7f?0x7e:0x7f):v);
    const wrongResult = declared.results.length===0 ? [0x7f] : [];
    for(const [label,override] of [
      ['parameter',{params:wrongParam}],
      ['return',{results:wrongResult}],
    ]) {
      const binary=moduleWithImports([{field,...override}]);
      assert.throws(()=>validate_actor_guest_imports(binary),
        /E_ACTOR_IMPORT_SIGNATURE/,field+':'+label);
    }
  }
});

test('actor_send cannot alias a single 64-bit integer or swap argument order',()=>{
  for(const params of [[0x7e],[0x7e,0x7f],[0x7f,0x7f],[0x7f,0x7e,0x7f]]) {
    const bytes=moduleWithImports([{field:'actor_send',params}]);
    assert.throws(()=>validate_actor_guest_imports(bytes),
      /E_ACTOR_IMPORT_SIGNATURE/,JSON.stringify(params));
  }
});

test('actor_join cannot return i32, i64, or accept an i64 actor handle',()=>{
  for(const descriptor of [
    {results:[0x7f]},{results:[0x7e]},{params:[0x7e]},
  ]) {
    assert.throws(
      ()=>validate_actor_guest_imports(
        moduleWithImports([{field:'actor_join',...descriptor}])),
      /E_ACTOR_IMPORT_SIGNATURE/);
  }
});

test('custom sections between type and import do not hide valid ABI',()=>{
  const input=moduleWithImports([
    {field:'print_newline'},
    {field:'actor_send'},
    {field:'actor_join'},
  ],{customBeforeImport:true});
  assert.deepEqual(
    validate_actor_guest_imports(input).map(x=>x.name),
    ['print_newline','actor_send','actor_join']);
});

test('a WebAssembly.Module object without matching source bytes fails closed',()=>{
  const bytes=moduleWithImports([{field:'actor_send'}]);
  const module=new WebAssembly.Module(bytes);
  assert.throws(()=>validate_actor_guest_imports(module),/E_ACTOR_IMPORT_BYTES/);
  assert.throws(()=>validate_actor_guest_imports({module,bytes}),/E_ACTOR_IMPORT_BYTES/);
  assert.throws(()=>validate_actor_guest_imports(null),/E_ACTOR_IMPORT_BYTES/);
});

test('Wasm module with absent type section but no imports is valid',()=>{
  const bytes=new Uint8Array([0,97,115,109,1,0,0,0]);
  assert.deepEqual(validate_actor_guest_imports(bytes),[]);
});

test('noncanonical ABI values cannot be smuggled via a type alias',()=>{
  const bytes=moduleWithImports([
    {field:'print_i64',params:[0x7f]},
    {field:'print_bool',params:[0x7f]},
  ]);
  assert.throws(()=>validate_actor_guest_imports(bytes),/E_ACTOR_IMPORT_SIGNATURE/);
});

test('a copied buffer view with offset still validates only the targeted module',()=>{
  const inner=moduleWithImports([{field:'actor_send'}]);
  const padding=new Uint8Array(inner.length+32);
  padding.fill(0xa5);
  padding.set(inner,16);
  const slice=padding.subarray(16,16+inner.length);
  assert.deepEqual(
    validate_actor_guest_imports(slice).map(x=>x.name),['actor_send']);
  assert.throws(()=>validate_actor_guest_imports(padding),
    /(E_ACTOR_IMPORT_BINARY|CompileError)/);
});


test('supervisor_create accepts only one i32 and returns one i32 handle',()=>{
  for(const altered of [
    {params:[]},{params:[0x7e]},{params:[0x7f,0x7f]},
    {results:[]},{results:[0x7e]},{results:[0x7f,0x7f]},
  ]){
    const bytes=moduleWithImports([{field:'supervisor_create',...altered}]);
    assert.throws(()=>validate_actor_guest_imports(bytes),
      /E_ACTOR_IMPORT_SIGNATURE/,JSON.stringify(altered));
  }
});

test('supervisor_watch requires two ordered i32 handles and returns a status i32',()=>{
  for(const altered of [
    {params:[]},{params:[0x7f]},{params:[0x7e,0x7f]},
    {params:[0x7f,0x7e]},{params:[0x7f,0x7f,0x7f]},
    {results:[]},{results:[0x7e]},{results:[0x7f,0x7f]},
  ]){
    const bytes=moduleWithImports([{field:'supervisor_watch',...altered}]);
    assert.throws(()=>validate_actor_guest_imports(bytes),
      /E_ACTOR_IMPORT_SIGNATURE/,JSON.stringify(altered));
  }
});

test('supervisor imports cannot be duplicated or passed under an attacker module',()=>{
  for(const field of ['supervisor_create','supervisor_watch']){
    assert.throws(()=>validate_actor_guest_imports(
      moduleWithImports([{field},{field}])),/E_ACTOR_IMPORT_DUPLICATE/);
    assert.throws(()=>validate_actor_guest_imports(
      moduleWithImports([{field,module:'env'}])),/E_ACTOR_IMPORT_NAME/);
    assert.throws(()=>validate_actor_guest_imports(
      moduleWithImports([{field,kind:'global'}])),/E_ACTOR_IMPORT_KIND/);
  }
});
