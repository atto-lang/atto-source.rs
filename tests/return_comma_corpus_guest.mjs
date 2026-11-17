// Executable per-vector conformance: exact-head native compiler vs wasm32 AOT compiler.
// No parser-only receipts, and deliberately unsupported tuple cases never count as passes.
// Usage: ATTOC=... ATTO_COMPILER_WASM=... ATTO_CONTRACT=... node tests/return_comma_corpus_guest.mjs
import assert from 'node:assert/strict';
import {readFile, mkdtemp, writeFile, rm, stat} from 'node:fs/promises';
import {createHash} from 'node:crypto';
import {spawnSync} from 'node:child_process';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';

const compiler=process.env.ATTOC, aot=process.env.ATTO_COMPILER_WASM,
      contract=process.env.ATTO_CONTRACT;
assert.ok(compiler&&aot&&contract,'ATTOC, ATTO_COMPILER_WASM and ATTO_CONTRACT required');
const suite=JSON.parse(await readFile(resolve(contract),'utf8'));
assert.equal(suite.corpusRevision,2);
assert.equal(suite.cases.length,37);
const compilerBytes=await readFile(resolve(aot));
assert.ok(WebAssembly.validate(compilerBytes),'AOT compiler is invalid Wasm');
const compilerModule=new WebAssembly.Module(compilerBytes);
assert.equal(WebAssembly.Module.imports(compilerModule).length,0,'AOT imports undeclared host capabilities');
const host=new WebAssembly.Instance(compilerModule,{}).exports;
assert.equal(host.abi_version(),2,'compiler ABI version mismatch');
const encoder=new TextEncoder(),decoder=new TextDecoder('utf8',{fatal:true});
const tmp=await mkdtemp(join(tmpdir(),'atto-return-comma-corpus-'));
const rows=[];
function sha(bytes) {return createHash('sha256').update(bytes).digest('hex');}
function compileInAot(source) {
  const input=encoder.encode(source);
  assert.ok(input.length<=host.input_capacity(),'fixture exceeds AOT compiler buffer');
  new Uint8Array(host.memory.buffer,host.input_ptr(),input.length).set(input);
  const rc=host.compile(input.length);
  if(rc===0) {
    const guest=Buffer.from(new Uint8Array(host.memory.buffer,host.result_ptr(),host.result_length()).slice());
    assert.ok(WebAssembly.validate(guest),'invalid AOT-generated guest');
    return {ok:true,guest};
  }
  assert.equal(host.result_length(),0,'AOT error emitted an artifact');
  const mem=new Uint8Array(host.memory.buffer),start=host.error_ptr();
  let end=start;
  while(end<mem.length&&end<start+8192&&mem[end]!==0)end++;
  assert.ok(end<mem.length&&end<start+8192,'AOT diagnostic unterminated');
  const message=decoder.decode(mem.subarray(start,end));
  return {ok:false,code:message.match(/\bE_[A-Z0-9_]+\b/)?.[0],message};
}
function runGuest(guest) {
  assert.ok(WebAssembly.validate(guest));
  const mod=new WebAssembly.Module(guest);
  assert.equal(WebAssembly.Module.imports(mod).length,0,'unexpected guest imports');
  const e=new WebAssembly.Instance(mod,{}).exports;
  assert.equal(typeof e.main,'function','no main');
  const out=e.main();
  assert.equal(e.panic_code?.value,0,'guest panic');
  return out;
}
try {
  for(const c of suite.cases) {
    const file=join(tmp,c.id+'.atto'),output=join(tmp,c.id+'.wasm');
    await writeFile(file,c.source,'utf8');
    const native=spawnSync(resolve(compiler),[file,'-o',output],{
      encoding:'utf8',timeout:30_000,maxBuffer:1<<20});
    assert.ok(!native.error,c.id+': compiler launch error '+String(native.error));
    const wasm=compileInAot(c.source);
    const code=(native.stderr||'').match(/\bE_[A-Z0-9_]+\b/)?.[0];
    const row={id:c.id,phase:c.phase,expect:c.expect,
      source_sha256:sha(Buffer.from(c.source)),native_exit:native.status,
      native_code:code,wasm_host_ok:wasm.ok,wasm_host_code:wasm.code,
      status:'failed'};
    try {
      if(c.expect==='reject'){
        assert.equal(native.status,1,c.id+': invalid source accepted on native');
        assert.equal(wasm.ok,false,c.id+': invalid source accepted in browser');
        await assert.rejects(stat(output),{code:'ENOENT'},c.id+': invalid artifact emitted');
        assert.ok(code,c.id+': native error lacks E_CODE');
        assert.equal(code,wasm.code,c.id+': diagnostic diverges between hosts');
        if(c.diagnostic==='effect-check'){
          assert.ok(/E_(?:EFFECT|PURE|SUSPENDS|THROWS)/.test(code),
            c.id+': wrong rejection cause '+code);
        }else{
          assert.equal(code,c.diagnostic,c.id+': wrong diagnostic category');
        }
        row.status='passed';
      }else if(c.phase==='execute'){
        assert.equal(native.status,0,c.id+': native compiler rejected: '+native.stderr);
        assert.equal(wasm.ok,true,c.id+': AOT compiler rejected: '+wasm.message);
        const guest=await readFile(output);
        assert.ok(guest.equals(wasm.guest),c.id+': native/AOT guest bytes differ');
        const actual=runGuest(guest);
        assert.equal(actual,BigInt(c.expectedReturn),c.id+': wrong guest result');
        row.return_value=String(actual);
        row.guest_sha256=sha(guest);
        row.status='passed';
      }else{
        // Current Rust browser compiler deliberately lacks tuple codegen.
        // Never relabel a rejected target-language positive case as passed.
        if(native.status===0&&wasm.ok){
          const guest=await readFile(output);
          assert.ok(guest.equals(wasm.guest),c.id+': compiled outputs differ');
          row.status='passed';
        }else{
          assert.equal(native.status,1,c.id+': inconsistent target-admission hosts');
          assert.equal(wasm.ok,false,c.id+': inconsistent target-admission hosts');
          assert.equal(code,wasm.code,c.id+': unsupported diagnostic mismatch');
          assert.ok(['E_UNSUPPORTED','E_UNSUPPORTED_TYPE'].includes(code),
            c.id+': unexpected target positive rejection '+code);
          await assert.rejects(stat(output),{code:'ENOENT'});
          row.status='unsupported';
          row.reason=code;
        }
      }
    }catch(e){row.detail=String(e?.stack||e);}
    rows.push(row);
  }
}finally{await rm(tmp,{recursive:true,force:true});}
const summary={
 corpusRevision:suite.corpusRevision,
 source_corpus_sha256:sha(Buffer.from(JSON.stringify(suite))),
 total:rows.length,
 passed:rows.filter(x=>x.status==='passed').length,
 unsupported:rows.filter(x=>x.status==='unsupported').length,
 failed:rows.filter(x=>x.status==='failed').length,
 cases:rows
};
console.log('ATTO_RETURN_CORPUS_RECEIPT='+JSON.stringify(summary));
assert.equal(summary.failed,0,'conformance vectors contain incorrect compiler or normative expectations');
assert.ok(summary.passed>=34,'unexpectedly few executed corpus cases');
assert.equal(summary.passed+summary.unsupported,37);
