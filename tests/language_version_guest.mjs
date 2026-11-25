// Real native / wasm32 compiler proof of versioned callable-body admission.
// No JavaScript interpreter of Atto semantics; guests execute in WebAssembly.
import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {mkdtemp, writeFile, readFile, rm, stat} from 'node:fs/promises';
import {resolve, join} from 'node:path';
import {tmpdir} from 'node:os';

const nativeCompiler = resolve(process.env.ATTOC || 'target/release/attoc');
const wasmCompiler = resolve(process.env.ATTO_COMPILER_WASM ||
  'target/wasm32-unknown-unknown/release/atto_compiler.wasm');
const bytes = await readFile(wasmCompiler);
assert.ok(WebAssembly.validate(bytes));
const m = new WebAssembly.Module(bytes);
assert.deepEqual(WebAssembly.Module.imports(m), [], 'compiler must not import host capabilities');
const abi = new WebAssembly.Instance(m, {}).exports;
assert.equal(abi.abi_version(), 2, 'existing browser ABI remains stable');
assert.equal(typeof abi.compile, 'function');
assert.equal(typeof abi.compile_v2, 'function', 'strict opt-in ABI must exist');

function compileBrowser(source, version) {
  const input = new TextEncoder().encode(source);
  assert.ok(input.length < abi.input_capacity());
  new Uint8Array(abi.memory.buffer, abi.input_ptr(), input.length).set(input);
  const status = (version === 1 ? abi.compile : abi.compile_v2)(input.length);
  if(status !== 0) {
    assert.equal(abi.result_length(), 0, 'rejected browser source leaked an artifact');
    const data = new Uint8Array(abi.memory.buffer);
    const start = abi.error_ptr();
    let end = start;
    while (end < data.length && end < start + 512 && data[end] !== 0) end++;
    assert.ok(end < start + 512, 'unterminated error');
    const message = new TextDecoder('utf-8', {fatal:true}).decode(data.subarray(start,end));
    return {ok:false,code:message.split(':')[0],message};
  }
  const guest = Buffer.from(new Uint8Array(abi.memory.buffer,
    abi.result_ptr(),abi.result_length()).slice());
  assert.ok(WebAssembly.validate(guest));
  return {ok:true,guest};
}

function execute(bytes) {
  const module = new WebAssembly.Module(bytes);
  assert.deepEqual(WebAssembly.Module.imports(module), [], 'numeric guest requires no host capability');
  const guest = new WebAssembly.Instance(module, {}).exports;
  assert.equal(guest.panic_code?.value, 0, 'guest panicked');
  return guest.main();
}

const cases = [
  {id:'legacy-unmarked-comma', version:1, src:'fnc f() -> int const n: int = 40, n + 2 end pub fnc main(): int { return f(); }', return:42},
  {id:'v2-reject-unmarked-comma', version:2, src:'fnc f() -> int const n: int = 40, n + 2 end pub fnc main(): int { return f(); }', reject:'E_BODY_VERSION'},
  {id:'v1-marked-comma', version:1, src:'fnc f() -> int :: const n: int = 40, n + 2 end pub fnc main(): int { return f(); }', return:42},
  {id:'v2-marked-comma', version:2, src:'fnc f() -> int :: const n: int = 40, n + 2 end pub fnc main(): int { return f(); }', return:42},
  {id:'v1-statement', version:1, src:'fnc f() -> int const n: int = 40; ^ n + 2; end pub fnc main(): int { return f(); }', return:42},
  {id:'v2-statement', version:2, src:'fnc f() -> int const n: int = 40; ^ n + 2; end pub fnc main(): int { return f(); }', return:42},
  {id:'legacy-nested-call-comma', version:1, src:'fnc add(int a,int b): int { return a+b; } fnc f() -> int add(20,22) end pub fnc main(): int { return f(); }', return:42},
  {id:'v2-nested-call-not-return', version:2, src:'fnc add(int a,int b): int { return a+b; } fnc f() -> int add(20,22) end pub fnc main(): int { return f(); }', reject:true},
  {id:'v1-seq-mutates', version:1, src:'pub fnc main(): int { let int z = 2; return seq(z += 5, z * 6); }', return:42},
  {id:'v2-seq-mutates', version:2, src:'pub fnc main(): int { let int z = 2; return seq(z += 5, z * 6); }', return:42},
  {id:'v2-explicit-class', version:2, src:'define class Foo as pub value() -> int :: const n: int = 40, n + 2 end end pub fnc main(): int { const f = new Foo(); return f.value(); }', return:42},
  {id:'v1-implicit-class', version:1, src:'define class Foo as pub value() -> int const n: int = 40, n + 2 end end pub fnc main(): int { const f = new Foo(); return f.value(); }', return:42},
  {id:'v1-unmarked-out-of-line-mtd', version:1, src:'define class C as final int seed: 40; end pub mtd (C self) value() -> int let int x = self.seed, x + 2 end pub fnc main(): int { val c = new C(); return c.value(); }', return:42},
  {id:'v2-reject-unmarked-out-of-line-mtd', version:2, src:'define class C as final int seed: 40; end pub mtd (C self) value() -> int let int x = self.seed, x + 2 end pub fnc main(): int { val c = new C(); return c.value(); }', reject:'E_BODY_VERSION'},
  {id:'v2-marked-out-of-line-mtd', version:2, src:'define class C as final int seed: 40; end pub mtd (C self) value() -> int :: let int x = self.seed, x + 2 end pub fnc main(): int { val c = new C(); return c.value(); }', return:42},
  {id:'v1-explicit-return-comma',version:1,src:'fnc f() -> int return 1, 42; end pub fnc main(): int { return f(); }',return:42},
  {id:'v2-explicit-return-comma',version:2,src:'fnc f() -> int return 1, 42; end pub fnc main(): int { return f(); }',return:42},
  {id:'v1-seq-initializer',version:1,src:'fnc f() -> int const int n = seq(1, 42); return n; end pub fnc main(): int { return f(); }',return:42},
  {id:'v2-seq-initializer',version:2,src:'fnc f() -> int const int n = seq(1, 42); return n; end pub fnc main(): int { return f(); }',return:42},
  {id:'v2-mtd-return-comma',version:2,src:'define class C as final int seed: 40; end pub mtd (C self) value() -> int return 1, self.seed + 2; end pub fnc main(): int { val c = new C(); return c.value(); }',return:42},
];

const tmp = await mkdtemp(join(tmpdir(), 'atto-version-gate-'));
let accepted = 0, rejected = 0;
try {
  for(const t of cases) {
    const input = join(tmp,t.id+'.atto'), output = join(tmp,t.id+'.wasm');
    await writeFile(input,t.src,'utf8');
    const args = ['--language-version',String(t.version),input,'-o',output];
    const native = spawnSync(nativeCompiler,args,{encoding:'utf8',timeout:15000,maxBuffer:1<<20});
    assert.equal(native.error, undefined, t.id);
    const browser = compileBrowser(t.src,t.version);
    if(t.reject) {
      assert.equal(native.status,1, `native should reject ${t.id}: ${native.stderr}`);
      assert.equal(browser.ok,false, `browser accepted invalid ${t.id}`);
      if(typeof t.reject === 'string') {
        assert.ok(native.stderr.includes(t.reject), `native wrong diagnostic ${t.id}`);
        assert.equal(browser.code,t.reject);
      }
      await assert.rejects(()=>stat(output),{code:'ENOENT'});
      rejected++;
      console.log('VERSION_REJECT',t.id,browser.code);
      continue;
    }
    assert.equal(native.status,0, `native compiler failed ${t.id}: ${native.stderr}`);
    assert.equal(browser.ok,true, `browser rejected ${t.id}: ${browser.message}`);
    const compiled = await readFile(output);
    assert.deepEqual(compiled,browser.guest, `native/browser binary mismatch: ${t.id}`);
    assert.equal(execute(compiled),BigInt(t.return), `guest returned incorrect value: ${t.id}`);
    accepted++;
    console.log('VERSION_GUEST',t.id,t.return);
  }
  // CLI errors MUST be distinct from source-language compile failures.
  for(const bad of ['0','3','2026','nope']) {
    const p = spawnSync(nativeCompiler,['--language-version',bad,'x.atto'],
      {encoding:'utf8'});
    assert.equal(p.status,2,'unknown edition must never default to another grammar');
  }
  const explicit = await readFile(join(tmp,'v1-marked-comma.wasm'));
  const basic = spawnSync(nativeCompiler,[join(tmp,'v1-marked-comma.atto'),'-o',join(tmp,'default.wasm')],
    {encoding:'utf8'});
  assert.equal(basic.status,0,'default CLI legacy admission');
  assert.deepEqual(await readFile(join(tmp,'default.wasm')),explicit,
    'default CLI version must be 1');
  console.log('VERSION_GATE_RECEIPT',JSON.stringify({accepted,rejected,
    byte_parity:accepted,executed_guests:accepted,
    invalid_selector_rejections:4,default_legacy:true}));
} finally {
  await rm(tmp,{recursive:true,force:true});
}
