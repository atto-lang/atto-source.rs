// Runs native-compiler-generated Wasm guests for all currently implemented callable forms.
// In particular, this checks *execution* rather than just compiler acceptance.
import assert from 'node:assert/strict';
import { readFile, writeFile, mkdtemp, rm, stat } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const attoc = process.env.ATTOC || fileURLToPath(new URL('../target/debug/attoc', import.meta.url));
const fixtures = [
  '165-callable-five-function-forms.atto',
  '166-callable-five-routine-forms.atto',
  '167-callable-five-inline-method-forms.atto',
  '168-callable-end-nested-arg-commas.atto',
  '173-class-mtd-executable.atto',
];
const negative = [
  '39-end-form-body.atto',
  '165-callable-trailing-comma.atto',
  '166-callable-colon-colon-brace.atto',
  '167-callable-brace-comma.atto',
  '168-callable-nested-brace-comma.atto',
  '169-callable-colon-arrow-end.atto',
];
const allowed = new Set(['print_i64','print_bool','print_str','print_newline','print_f64','actor_spawn','actor_send']);
const imports = {
  atto: {
    print_i64: () => { throw Error('unexpected printing'); },
    print_bool: () => { throw Error('unexpected printing'); },
    print_str: () => { throw Error('unexpected printing'); },
    print_newline: () => { throw Error('unexpected printing'); },
    print_f64: () => { throw Error('unexpected printing'); },
    actor_spawn: () => { throw Error('unexpected actor spawn'); },
    actor_send: () => { throw Error('unexpected actor send'); },
  }
};
const run = (source, out) => spawnSync(attoc, [source, '-o', out], { encoding: 'utf8', timeout: 30_000, maxBuffer: 1 << 20 });
const work = await mkdtemp(join(tmpdir(), 'atto-callable-'));
try {
  for (const name of fixtures) {
    const path = fileURLToPath(new URL('./fixtures/pass/' + name, import.meta.url));
    const source = await readFile(path, 'utf8');
    const want = source.match(/^\/\/ expect-exit: (\d+)/m)?.[1];
    assert.ok(want, name + ': expected exit header missing');
    const out = join(work, name + '.wasm');
    const built = run(path, out);
    assert.equal(built.status, 0, name + ': attoc failed: ' + built.stderr);
    const wasm = await readFile(out);
    assert.ok(WebAssembly.validate(wasm), name + ': invalid guest');
    const module = new WebAssembly.Module(wasm);
    for (const imp of WebAssembly.Module.imports(module)) {
      assert.equal(imp.module, 'atto', name + ': wrong import module');
      assert.ok(allowed.has(imp.name), name + ': unexpected import ' + imp.name);
    }
    const guest = new WebAssembly.Instance(module, imports);
    const actual = guest.exports.main();
    assert.equal(actual, BigInt(want), name + ': wrong guest return value');
    assert.equal(guest.exports.panic_code.value, 0, name + ': unexpected panic');
    console.log('PASS guest execution', name, String(actual));
  }
  // Separately compile and execute each current-main intrinsic spelling.
  for (const [index,sig,lambda,args,tuple] of [
    [0,'fnc(int,int)->int','|a,b| : int -> a+b','20,22','(20,22)'],
    [1,'fnc(int,bool)->int','|a,b| : int -> a+2','40,true','(40,true)'],
    [2,'fnc(int)->int','|a| : int -> a+2','40','(40,)'],
    [3,'fnc()->int','|| : int -> 42','','()'],
  ]) {
    for (const binding of ['val','const']) {
      let baseline;
      for (const [form,expression] of [
        ['direct','f('+args+')'],
        ['call','f.call('+args+')'],
        ['apply','f.apply('+tuple+')'],
      ]) {
        const source = join(work,'intrinsic-'+index+'-'+binding+'-'+form+'.atto');
        const out = source+'.wasm';
        await writeFile(source,'pub fnc main():int { '+binding+' f: '+sig+' = '+lambda+'; return '+expression+'; }');
        const built = run(source,out);
        assert.equal(built.status,0,source+': '+built.stderr);
        const bytes = await readFile(out);
        assert.ok(WebAssembly.validate(bytes),source);
        if (baseline) assert.deepEqual(bytes,baseline,source+': byte drift');
        else baseline=bytes;
        const module = new WebAssembly.Module(bytes);
        for (const imp of WebAssembly.Module.imports(module)) {
          assert.equal(imp.module,'atto');
          assert.ok(allowed.has(imp.name));
        }
        const guest = new WebAssembly.Instance(module,imports);
        assert.equal(guest.exports.main(),42n,source);
        assert.equal(guest.exports.panic_code.value,0,source);
        console.log('PASS callable guest',index,binding,form);
      }
    }
  }
  for (const [index,expression,diagnostic] of [
    [0,'f.call(20)','E_ARITY'],
    [1,'f.apply((20,true))','E_TYPE'],
    [2,'f.apply((20,))','E_ARITY'],
    [3,'f.apply([20,22])','E_UNSUPPORTED'],
    [4,'f.bind(20)','E_UNSUPPORTED'],
  ]) {
    const source = join(work,'intrinsic-negative-'+index+'.atto');
    const out = source+'.wasm';
    await writeFile(source,'pub fnc main():int { val f: fnc(int,int)->int = |a,b| : int -> a+b; return '+expression+'; }');
    const built = run(source,out);
    assert.equal(built.status,1,source+': '+built.stderr);
    assert.ok(built.stderr.includes(diagnostic),source+': '+built.stderr);
    await assert.rejects(stat(out),{code:'ENOENT'});
  }
  // Sibling closures share no environment storage. Test the identical
  // compiler output with each intrinsic independently and in combination.
  const siblings = 'pub fnc main():int { val offset = 2; val plus = |n| : int -> n+offset; val double: fnc(int)->int = |n| : int -> n*2; val seven: fnc()->int = || : int -> 7; return ';
  let siblingBaseline;
  for (const [form,expression] of [
    ['direct','plus(3)+double(15)+seven()'],
    ['call','plus.call(3)+double.call(15)+seven.call()'],
    ['apply','plus(3)+double.apply((15,))+seven.apply(())'],
    ['mixed','plus.call(3)+double.apply((15,))+seven.call()'],
  ]) {
    const source = join(work,'sibling-'+form+'.atto');
    const out = source+'.wasm';
    await writeFile(source,siblings+expression+'; }');
    const built = run(source,out);
    assert.equal(built.status,0,source+': '+built.stderr);
    const bytes = await readFile(out);
    assert.ok(WebAssembly.validate(bytes),source);
    if (siblingBaseline) assert.deepEqual(bytes,siblingBaseline,source+': byte drift');
    else siblingBaseline = bytes;
    const module = new WebAssembly.Module(bytes);
    for (const imp of WebAssembly.Module.imports(module)) {
      assert.equal(imp.module,'atto');
      assert.ok(allowed.has(imp.name));
    }
    const guest = new WebAssembly.Instance(module,imports);
    assert.equal(guest.exports.main(),42n,source);
    assert.equal(guest.exports.panic_code.value,0,source);
    console.log('PASS sibling callable guest',form);
  }
  for (const [index,expression] of [
    [0,'later(1)'],
    [1,'later.call(1)'],
    [2,'later.apply((1,))'],
    [3,'later'],
  ]) {
    const source = join(work,'sibling-negative-'+index+'.atto');
    const out = source+'.wasm';
    await writeFile(source,'fnc later(int n):int { return n*100; } pub fnc main():int { val first = |n| : int -> '+expression+'; val later = |n| : int -> n+1; return first(1)+later(1); }');
    const built = run(source,out);
    assert.equal(built.status,1,source+': '+built.stderr);
    assert.ok(built.stderr.includes('E_UNSUPPORTED'),source+': '+built.stderr);
    await assert.rejects(stat(out),{code:'ENOENT'});
  }
  // Named fnc stage proof: actual compiler and bounded separate Wasm process.
  // The expensive fib(55) source is compiled in Rust tests, not run in smoke CI.
  const namedStage = n => 'pub fnc main(): int { return '+n+' |> fnc fib(int v): int { ^ match v over on 0 -> 0, on 1 -> 1, on _ -> fib(v - 2) + fib(v - 1) end }; }';
  const namedCases = [
    ['zero',namedStage(0),'0'],
    ['one',namedStage(1),'1'],
    ['ten',namedStage(10),'55'],
    ['twenty',namedStage(20),'6765'],
    ['semicolon',namedStage(10).replace(' end }',' end; }'),'55'],
    ['homonym','fnc fib(int v): int { return 999; } '+namedStage(10),'55'],
    ['two-identities','pub fnc main(): int { return 1 |> fnc f(int v): int { return v+2; } |> fnc f(int v): int { return v*14; }; }','42'],
    ['stack-quota','pub fnc main(): int { return 2048 |> fnc f(int v): int { return f(v-1); }; }','stack'],
  ];
  for(const [id,src,want] of namedCases) {
    const source=join(work,'named-stage-'+id+'.atto'),out=source+'.wasm';
    await writeFile(source,src);
    const built=run(source,out);
    assert.equal(built.error,undefined,id+': compile subprocess failure');
    assert.equal(built.status,0,id+': '+built.stderr);
    const worker=`import {readFileSync} from 'node:fs';
const bytes=readFileSync(process.argv[1]);if(!WebAssembly.validate(bytes))throw Error('invalid Wasm');
const m=new WebAssembly.Module(bytes);if(WebAssembly.Module.imports(m).length)throw Error('unexpected host imports');
const i=new WebAssembly.Instance(m,{});
try{const n=i.exports.main();if(typeof n!=='bigint'||i.exports.panic_code.value!==0)throw Error('wrong result ABI');console.log(String(n));}
catch(e){if(!(e instanceof WebAssembly.RuntimeError)||i.exports.panic_code.value!==6)throw e;console.log('stack');}`;
    const guest=spawnSync(process.execPath,['--input-type=module','-e',worker,out],{encoding:'utf8',timeout:5000,maxBuffer:1<<20});
    assert.equal(guest.error,undefined,id+': guest subprocess failure');
    assert.equal(guest.status,0,id+': '+guest.stderr);
    assert.equal(guest.stdout,want+'\n',id+': wrong guest result');
    console.log('PASS named recursive guest',id,want);
  }
  for(const [id,src,code] of [
    ['escape','pub fnc main(): int { val x = 1 |> fnc f(int v): int { return v; }; return f(x); }','E_UNBOUND'],
    ['capture','pub fnc main(): int { val x = 1; return 1 |> fnc f(int v): int { return v+x; }; }','E_UNSUPPORTED'],
    ['arity','pub fnc main(): int { return 1 |> fnc f(int v): int { return f(v,v); }; }','E_ARITY'],
  ]) {
    const source=join(work,'named-stage-negative-'+id+'.atto'),out=source+'.wasm';
    await writeFile(source,src);
    const built=run(source,out);
    assert.equal(built.error,undefined,id+': compile infrastructure');
    assert.equal(built.status,1,id+': '+built.stderr);
    assert.match(built.stderr,new RegExp('\\b'+code+':'));
    await assert.rejects(stat(out),{code:'ENOENT'});
  }
  for (const name of negative) {
    const path = fileURLToPath(new URL('./fixtures/fail/' + name, import.meta.url));
    const source = await readFile(path, 'utf8');
    const want = source.match(/^\/\/ expect-error: (E_\w+)/m)?.[1];
    assert.ok(want, name + ': expected error header missing');
    const output = join(work, name + '.wasm');
    const built = run(path, output);
    assert.equal(built.status, 1, name + ': expected compiler rejection');
    assert.ok(built.stderr.includes(want), name + ': missing ' + want + ': ' + built.stderr);
    await assert.rejects(stat(output), {code:'ENOENT'}, name + ': rejected source emitted an artifact');
    console.log('PASS expected diagnostic', name, want);
  }
} finally {
  await rm(work, { recursive: true, force: true });
}
