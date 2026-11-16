// Exact source-native and wasm32-host audit of atto-spec#35's proposed 49 cases.
// This is NOT a claim of language-version migration or other-backend support.
// Run after both attoc and atto_compiler.wasm have been built in CI.
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {mkdtemp, readFile, writeFile, stat, rm, mkdir} from 'node:fs/promises';
import {spawnSync} from 'node:child_process';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';

const path = new URL('./contracts/caret-return-sequences-v1.json', import.meta.url);
const bytes = await readFile(path);
const sha = createHash('sha1').update('blob ' + bytes.length + '\0').update(bytes).digest('hex');
assert.equal(sha, '0fad4e5a6047e748f657c93cb79d0e4860416b71',
  'Spec snapshot changed without updating upstream Git-blob receipt');
const contract = JSON.parse(bytes.toString('utf8'));
assert.equal(contract.cases.length, 49);
assert.equal(contract.corpusRevision, 3);
const ids = new Set();
for(const c of contract.cases) {
  assert.ok(!ids.has(c.id), 'duplicate normative case '+c.id); ids.add(c.id);
  assert.ok(['accept','reject'].includes(c.expect));
  assert.ok(['execute','compile','parse','typecheck'].includes(c.phase));
}
const binary = resolve(process.env.ATTOC || 'target/release/attoc');
const wasmCompilerFile = resolve(process.env.ATTO_COMPILER_WASM ||
  'target/wasm32-unknown-unknown/release/atto_compiler.wasm');
const compilerBytes = await readFile(wasmCompilerFile);
assert.ok(WebAssembly.validate(compilerBytes), 'real wasm32 compiler is invalid');
const compilerModule = new WebAssembly.Module(compilerBytes);
assert.equal(WebAssembly.Module.imports(compilerModule).length, 0, 'compiler must not import host capabilities');
const api = new WebAssembly.Instance(compilerModule, {}).exports;
assert.equal(api.abi_version(),2);
const encoder = new TextEncoder(), decoder = new TextDecoder('utf8',{fatal:true});
function compileWasmHost(input, languageVersion = 2) {
  const b = encoder.encode(input);
  if(b.length > api.input_capacity()) return {ok:false,code:'E_SOURCE_LIMIT'};
  new Uint8Array(api.memory.buffer,api.input_ptr(),b.length).set(b);
  assert.equal(typeof api.compile_versioned,'function','versioned Wasm compiler ABI is required');
  const rc=api.compile_versioned(b.length,languageVersion);
  if(rc!==0){
    assert.equal(api.result_length(),0,'error emitted wasm artifact');
    const mem = new Uint8Array(api.memory.buffer);
    const start=api.error_ptr(); let end=start;
    while(end<mem.length && end<start+8192 && mem[end]!==0) end++;
    assert.ok(end<mem.length && end<start+8192,'unterminated compiler diagnostic');
    const message=decoder.decode(mem.subarray(start,end));
    return {ok:false,code:message.split(':')[0],message,
      line:api.error_line_number(),column:api.error_column_number()};
  }
  return {ok:true,wasm:Buffer.from(new Uint8Array(api.memory.buffer,api.result_ptr(),api.result_length()).slice())};
}
function guestRun(wasm){
  assert.ok(WebAssembly.validate(wasm), 'invalid Wasm guest');
  const module=new WebAssembly.Module(wasm);
  const known=new Set(['print_i64','print_bool','print_str','print_newline','print_f64',
    'actor_spawn','actor_send','state_register','state_enter','state_leave']);
  const imports={atto:{}};
  for(const imp of WebAssembly.Module.imports(module)){
    assert.equal(imp.module,'atto','guest import namespace is not allowlisted');
    assert.ok(imp.kind==='function' && known.has(imp.name),'guest imports undeclared capability '+imp.name);
    imports.atto[imp.name]=()=>{throw Error('unexpected guest host import: '+imp.name)};
  }
  const instance=new WebAssembly.Instance(module,imports);
  assert.equal(typeof instance.exports.main,'function');
  const actual=instance.exports.main();
  assert.equal(typeof actual,'bigint','guest main must produce i64');
  assert.equal(instance.exports.panic_code?.value,0,'unexpected guest panic');
  return actual;
}
const work=await mkdtemp(join(tmpdir(),'atto-spec-caret-'));
const rows=[];
try{
  for(const c of contract.cases) {
    const input=join(work,c.id+'.atto'), output=join(work,c.id+'.wasm');
    await writeFile(input,c.source,'utf8');
    const native=spawnSync(binary,[input,'-o',output,'--language-version','2'],{encoding:'utf8',timeout:30000,maxBuffer:1<<20});
    const wasmHost=compileWasmHost(c.source);
    const row={
      id:c.id,phase:c.phase,expect:c.expect,expectedReturn:c.expectedReturn,
      expectedDiagnosticCategory:c.diagnostic,sourceBlobSha:createHash('sha1')
        .update('blob '+Buffer.byteLength(c.source)+'\0').update(c.source).digest('hex'),
      nativeExit:native.status,wasmHostOk:wasmHost.ok,wasmHostCode:wasmHost.code,
      nativeDiagnostic:(native.stderr||'').slice(0,2048),
      diagnostic_category_checked:false,
      status:'failed',detail:'not evaluated'
    };
    try {
      if(native.error) throw native.error;
      const nativeCode=(native.stderr||'').match(/\bE_[A-Z0-9_]+\b/)?.[0];
      row.nativeCode=nativeCode;
      if(c.expect==='reject') {
        let found=false;
        try{await stat(output);found=true}catch(e){if(e.code!=='ENOENT')throw e;}
        assert.equal(found,false,'rejected source emitted native artifact');
        assert.equal(native.status,1,'native compiler accepted invalid spec program');
        assert.equal(wasmHost.ok,false,'wasm32 compiler accepted invalid spec program');
        assert.equal(nativeCode,wasmHost.code,'hosts disagree on rejected diagnostic code');
        // Rejection itself is verified. The prose diagnostic category is NOT
        // yet mapped to a stable E_CODE: keep diagnostic_category_checked=false.
        row.status=['E_UNSUPPORTED','E_UNSUPPORTED_TYPE'].includes(wasmHost.code)?'unsupported':'passed';
        row.detail=row.status==='passed'?'both hosts reject without artifact; code agrees (category unverified)':
          'both hosts reject via E_UNSUPPORTED';
      } else {
        if(native.status!==0 || !wasmHost.ok) {
          row.status=['E_UNSUPPORTED','E_UNSUPPORTED_TYPE'].includes(nativeCode) &&
            nativeCode===wasmHost.code?'unsupported':'failed';
          row.detail='expected acceptance; native/wasm compiler returned rejection';
        } else {
          const nativeBytes=await readFile(output);
          assert.ok(WebAssembly.validate(nativeBytes),'native compiler emitted invalid Wasm');
          assert.deepEqual(nativeBytes,wasmHost.wasm,'native and wasm32 compiler guest bytes differ');
          if(c.phase==='execute'){
            const actual=guestRun(wasmHost.wasm);
            row.actualReturn=String(actual);
            assert.equal(actual,BigInt(c.expectedReturn),'guest returned incorrect value');
            row.detail='native+wasm32 byte parity, executed guest returned expected value';
          } else row.detail='native+wasm32 byte parity; compile-only case not executed';
          row.status='passed';
        }
      }
    } catch(e) {
      row.status='failed'; row.detail=String(e?.message||e).slice(0,512);
    }
    rows.push(row);
    console.log('CARET_SPEC_CASE',row.id,row.status,row.detail);
    if(row.status!=='passed') console.log('CARET_SPEC_DIAGNOSTIC',row.id,
      'native='+String(row.nativeCode),'wasm='+String(wasmHost.code),
      row.nativeDiagnostic.replace(/\s+/g,' ').slice(0,400));
  }
}finally{await rm(work,{recursive:true,force:true});}
const summary=Object.fromEntries(['passed','failed','unsupported','not_executed'].map(k=>
  [k,rows.filter(x=>x.status===k).length]));
assert.equal(rows.length,49);
const report={schema:'atto.caret.spec-audit.v1',specPullRequest:'atto-lang/atto-spec#35',
  specGitBlob:sha,sourceVersioning:'explicit-v2-native-cli-and-wasm-abi',
  backendPair:'native-rust-attoc/wasm32-unknown-unknown-rust',
  diagnosticCategoryMapping:'unverified',
  compilerWasmSha256:createHash('sha256').update(compilerBytes).digest('hex'),
  summary,rows};
await mkdir('dist',{recursive:true});
await writeFile('dist/caret_spec_audit.json',JSON.stringify(report,null,2)+'\n','utf8');
console.log('CARET_SPEC_AUDIT',JSON.stringify(summary),
  'corpus_sha='+sha,'strict='+String(process.env.ATTO_CARET_ENFORCE_ALL==='1'));
if(process.env.ATTO_CARET_ENFORCE_ALL==='1'){
  if(summary.passed!==49)process.exitCode=1;
} else {
  // These 42 exact IDs passed real native+wasm32 CI (runs 43823854425 and
  // 43825249538). E_UNSUPPORTED_TYPE is not proof of type mismatch.
  // Keep each as a hard regression gate; all remaining proposed cases must
  // appear in the report as failed/unsupported until separately fixed.
  const priorPassed = new Set([
    "caret-scalar",
    "caret-no-whitespace-lexing",
    "caret-nested-if",
    "caret-nested-loop",
    "caret-method",
    "caret-return-match",
    "caret-void",
    "caret-return-seq",
    "caret-return-paren-sequence",
    "paren-sequence-nested",
    "paren-sequence-call-argument",
    "paren-sequence-outer-binding",
    "begin-sequence-equivalent",
    "caret-from-sequence-step",
    "reject-caret-infix",
    "reject-caret-as-initializer",
    "reject-caret-as-call-argument",
    "reject-caret-type-mismatch",
    "reject-caret-empty-nonvoid",
    "reject-caret-top-level",
    "reject-caret-xor-ambiguity",
    "reject-unterminated-paren-seq",
    "reject-trailing-seq-comma",
    "reject-empty-sequence",
    "reject-sequence-ending-binding",
    "reject-seq-no-implicit-return",
    "reject-sequence-duplicate-bind",
    "reject-sequence-binding-leak",
    "paren-comparison-not-sequence",
    "end-statement-method-explicit-return",
    "double-colon-method-implicit-return",
    "end-statement-free-fnc",
    "double-colon-free-fnc",
    "double-colon-nested-final-sequence",
    "reject-double-colon-caret-binding",
    "reject-unmarked-comma-body",
    "reject-unmarked-final-expr-implicit-return",
    "reject-double-colon-semicolon-body",
    "reject-double-colon-no-tail-expression",
    "reject-double-colon-braces",
    "reject-double-colon-after-colon-type",
    "double-colon-early-return-arm"
]);
  for(const id of priorPassed) {
    if(rows.find(x=>x.id===id)?.status!=='passed') {
      console.error('Previously passing normative vector regressed:',id);
      process.exitCode=1;
    }
  }
  assert.equal(priorPassed.size,42);
}


// In addition to the proposed v2 normative corpus, require actual executed
// v1/v2 compatibility for the same native and browser compiler builds.
// These regressions must never be counted as part of the 49 normative rows.
const versionMatrix = [
  {id:'v1-unmarked-free',version:1,accept:true,value:42,
    source:'pub fnc main() -> int let int x = 5, x + 37 end'},
  {id:'v1-explicit-colon-colon',version:1,accept:true,value:42,
    source:'pub fnc main() -> int :: let int x = 5, x + 37 end'},
  {id:'v2-explicit-colon-colon',version:2,accept:true,value:42,
    source:'pub fnc main() -> int :: let int x = 5, x + 37 end'},
  {id:'v2-unmarked-statements',version:2,accept:true,value:42,
    source:'pub fnc main() -> int const x: int = 5; ^ x + 37; end'},
  {id:'v1-unmarked-routine',version:1,accept:true,value:42,
    source:'routine answer() -> int let int x = 5, x + 37 end pub fnc main(): int { ^ answer(); }'},
  {id:'v1-unmarked-inline-method',version:1,accept:true,value:42,
    source:'define class C as pub answer() -> int let int x = 5, x + 37 end end pub fnc main(): int { val c = new C(); ^ c.answer(); }'},
  {id:'v1-unmarked-receiver',version:1,accept:true,value:42,
    source:'define class C as final int seed: 40; end pub mtd (self: C) answer() -> int let int x = self.seed, x + 2 end pub fnc main(): int { val c = new C(); ^ c.answer(); }'},
  {id:'v1-nested-call-commas',version:1,accept:true,value:42,
    source:'fnc add(int a, int b): int { ^ a + b; } pub fnc main() -> int let int x = 5, add(x, 37) end'},
  {id:'v2-reject-v1-unmarked-free',version:2,accept:false,diagnostic:'E_LANGUAGE_VERSION',
    source:'pub fnc main() -> int let int x = 5, x + 37 end'},
  {id:'v2-reject-v1-unmarked-routine',version:2,accept:false,diagnostic:'E_LANGUAGE_VERSION',
    source:'routine answer() -> int let int x = 5, x + 37 end pub fnc main(): int { ^ answer(); }'},
  {id:'v2-reject-v1-unmarked-inline-method',version:2,accept:false,diagnostic:'E_LANGUAGE_VERSION',
    source:'define class C as pub answer() -> int let int x = 5, x + 37 end end pub fnc main(): int { val c = new C(); ^ c.answer(); }'},
  {id:'v2-reject-v1-unmarked-receiver',version:2,accept:false,diagnostic:'E_LANGUAGE_VERSION',
    source:'define class C as final int seed: 40; end pub mtd (self: C) answer() -> int let int x = self.seed, x + 2 end pub fnc main(): int { val c = new C(); ^ c.answer(); }'},
  {id:'v2-reject-unmarked-with-nested-call',version:2,accept:false,diagnostic:'E_LANGUAGE_VERSION',
    source:'fnc add(int a, int b): int { ^ a + b; } pub fnc main() -> int let int x = 5, add(x, 37) end'},
  {id:'v2-reject-colon-colon-semicolon',version:2,accept:false,diagnostic:'E_SYNTAX',
    source:'pub fnc main() -> int :: const x: int = 5; x + 37 end'}
];
const versionRows=[];
const versionWork=await mkdtemp(join(tmpdir(),'atto-callable-versions-'));
try {
  for(const c of versionMatrix) {
    const input=join(versionWork,c.id+'.atto');
    const output=join(versionWork,c.id+'.wasm');
    await writeFile(input,c.source+'\n','utf8');
    const native=spawnSync(binary,[input,'-o',output,'--language-version',String(c.version)],
      {encoding:'utf8',timeout:30000,maxBuffer:1<<20});
    const wasmHost=compileWasmHost(c.source+'\n',c.version);
    const detail={id:c.id,version:c.version,accept:c.accept,
      nativeExit:native.status,wasmOk:wasmHost.ok,wasmCode:wasmHost.code,
      nativeCode:(native.stderr||'').match(/\bE_[A-Z0-9_]+\b/)?.[0],
      guestReturn:null,status:'failed'};
    try {
      assert.ifError(native.error);
      if(c.accept) {
        assert.equal(native.status,0,'native compiler rejected valid versioned source');
        assert.equal(wasmHost.ok,true,'browser compiler rejected valid versioned source');
        const nativeGuest=await readFile(output);
        assert.ok(WebAssembly.validate(nativeGuest),'invalid native guest');
        assert.deepEqual(nativeGuest,wasmHost.wasm,'native/browser compiler bytes differ');
        const actual=guestRun(wasmHost.wasm);
        detail.guestReturn=String(actual);
        assert.equal(actual,BigInt(c.value),'wrong guest result');
      } else {
        assert.equal(native.status,1,'native accepted invalid body mode');
        assert.equal(wasmHost.ok,false,'browser accepted invalid body mode');
        let exists=false;
        try {await stat(output);exists=true;}catch(e){if(e.code!=='ENOENT')throw e;}
        assert.equal(exists,false,'invalid source produced native artifact');
        assert.equal(detail.nativeCode,c.diagnostic,'wrong native diagnostic');
        assert.equal(wasmHost.code,c.diagnostic,'wrong browser diagnostic');
        assert.equal(api.result_length(),0,'failed browser compile retained a guest artifact');
      }
      detail.status='passed';
    } catch(err) {
      detail.error=String(err?.message||err).slice(0,512);
    }
    versionRows.push(detail);
    console.log('CALLABLE_VERSION_CASE',c.id,detail.status,detail.error||'');
  }
  // Reject unknown versions, no fallback to current default and no artifact.
  for(const unsupportedVersion of [0,3,4294967295]) {
    const bad=compileWasmHost('pub fnc main(): int { ^ 42; }',unsupportedVersion);
    assert.equal(bad.ok,false);
    assert.equal(bad.code,'E_LANGUAGE_VERSION');
    assert.equal(api.result_length(),0);
  }
  // Host ABI must reset state after both successful and failed compilation:
  // stale valid bytes must never be exposed as a failed compile artifact.
  for(const version of [1,2]) {
    const good = compileWasmHost('pub fnc main(): int { ^ 42; }',version);
    assert.equal(good.ok,true);
    assert.equal(guestRun(good.wasm),42n);
    const rejected = compileWasmHost('pub fnc main(): int { ^ missing_name; }',version);
    assert.equal(rejected.ok,false);
    assert.equal(rejected.code,'E_UNBOUND');
    assert.equal(api.result_length(),0,'failure leaked previous guest output');
    const recovered = compileWasmHost('pub fnc main(): int { ^ 42; }',version);
    assert.equal(recovered.ok,true,'ABI state contaminated next compilation');
    assert.deepEqual(recovered.wasm,good.wasm,'recovered output changed');
  }
  const invalid=spawnSync(binary,['--language-version','3',join(versionWork,'missing.atto')],
    {encoding:'utf8',timeout:30000,maxBuffer:1<<20});
  assert.equal(invalid.status,2,'CLI must reject unrecognized language version');
  assert.match(invalid.stderr,/E_LANGUAGE_VERSION/,'CLI must identify invalid grammar selection');
  for(const flag of ['0','4294967295','junk']) {
    const rejectedVersion=spawnSync(binary,['--language-version='+flag,
      join(versionWork,'missing.atto')],{encoding:'utf8',timeout:30000,maxBuffer:1<<20});
    assert.equal(rejectedVersion.status,2);
    assert.match(rejectedVersion.stderr,/E_LANGUAGE_VERSION/);
  }
  // Native file reads must be bounded to SOURCE_CAP+1 bytes before decoding:
  // giant user input should reject without emitting an output Wasm file.
  const bigInput=join(versionWork,'oversized.atto');
  const bigOutput=join(versionWork,'oversized.wasm');
  await writeFile(bigInput,' '.repeat(65537),'utf8');
  for(const version of [1,2]) {
    const nativeTooBig=spawnSync(binary,[bigInput,'-o',bigOutput,
      '--language-version',String(version)],{encoding:'utf8',timeout:30000,maxBuffer:1<<20});
    assert.equal(nativeTooBig.status,1);
    assert.match(nativeTooBig.stderr,/E_SOURCE_LIMIT/);
    let emitted=false;
    try {await stat(bigOutput);emitted=true;} catch(e) {if(e.code!=='ENOENT') throw e;}
    assert.equal(emitted,false,'oversized input emitted a guest');
  }
  const oversizedHost=compileWasmHost(' '.repeat(65537),1);
  assert.equal(oversizedHost.ok,false);
  assert.equal(oversizedHost.code,'E_SOURCE_LIMIT');
  assert.equal(api.result_length(),0);
} finally { await rm(versionWork,{recursive:true,force:true}); }
const versionReport={schema:'atto.callable-language-versions.v1',
  sourceVersions:['1','2'],sourceVersioning:'explicit-only',
  normativeCorpusBlobSha:sha,compilerWasmSha256:report.compilerWasmSha256,
  summary:{passed:versionRows.filter(x=>x.status==='passed').length,
           failed:versionRows.filter(x=>x.status==='failed').length},
  rows:versionRows};
await writeFile('dist/callable_version_matrix.json',JSON.stringify(versionReport,null,2)+'\n');
console.log('CALLABLE_VERSION_MATRIX',JSON.stringify(versionReport.summary));
assert.equal(versionReport.summary.failed,0,'versioned native/wasm execution or no-artifact proof failed');
assert.equal(versionReport.summary.passed,versionMatrix.length);
