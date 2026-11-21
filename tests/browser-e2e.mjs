// Actual Chromium + web workers + Rust-compiled compiler + guest Wasm.
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {spawn} from 'node:child_process';
import {createHash} from 'node:crypto';
import {mkdtemp,readFile,rm} from 'node:fs/promises';
import {join} from 'node:path';
import {tmpdir} from 'node:os';
const root=new URL('.',import.meta.url), tmp=await mkdtemp(join(tmpdir(),'atto-rust-browser-'));
const requests=[];
const files=new Map([
  ['/index.html',[new URL('./browser/index.html',import.meta.url),'text/html; charset=utf-8']],
  ['/worker.mjs',[new URL('./browser/worker.mjs',import.meta.url),'text/javascript; charset=utf-8']],
  ['/state-runtime.mjs',[new URL('./browser/state-runtime.mjs',import.meta.url),'text/javascript; charset=utf-8']],
  ['/atto_compiler.wasm',[new URL('../target/wasm32-unknown-unknown/release/atto_compiler.wasm',import.meta.url),'application/wasm']],
]);
const server=createServer(async(req,res)=>{
  requests.push({method:req.method,path:req.url});
  const entry=files.get(req.url||'');
  if(req.method!=='GET'||!entry){res.writeHead(404);res.end('Not found');return}
  try{const body=await readFile(entry[0]);res.writeHead(200,{'content-type':entry[1],'x-content-type-options':'nosniff','cache-control':'no-store'});res.end(body)}
  catch{res.writeHead(404);res.end('missing')}
});
const sleep=ms=>new Promise(r=>setTimeout(r,ms));
async function until(fn,label){
  let last;
  for(let i=0;i<400;i++){
    try{const value=await fn();if(value)return value;last=value}catch(e){last=e.message}
    await sleep(100);
  }
  throw new Error((typeof label==='function'?label():label)+' timed out: '+last);
}
let chrome,ws;
try{
  await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
  // A missing Rust-to-Wasm compiler artifact is a harness error, not a
  // forty-second guest-runtime timeout. Verify it before launching Chrome.
  const browserCompiler=await readFile(new URL('../target/wasm32-unknown-unknown/release/atto_compiler.wasm',import.meta.url));
  assert.ok(WebAssembly.validate(browserCompiler),'browser compiler Wasm missing or invalid');
  const browser=process.env.CHROME_BIN||'/usr/bin/google-chrome';
  chrome=spawn(browser,['--headless=new','--no-sandbox','--disable-gpu','--disable-dev-shm-usage','--no-first-run','--no-proxy-server','--disable-background-networking','--remote-debugging-port=0','--user-data-dir='+tmp,'about:blank'],{stdio:['ignore','ignore','pipe']});
  let stderr='';chrome.stderr.on('data',x=>stderr=(stderr+x.toString()).slice(-2000));
  const port=await until(async()=>{
    if(chrome.exitCode!==null)throw new Error('Chrome exited: '+stderr);
    try{const value=Number((await readFile(join(tmp,'DevToolsActivePort'),'utf8')).split('\n')[0]);return value>0?value:null}catch{return null}
  },'chromium port');
  const targets=await (await fetch('http://127.0.0.1:'+port+'/json/list')).json();
  const page=targets.find(x=>x.type==='page'&&x.webSocketDebuggerUrl);
  assert.ok(page,'page target exists');
  ws=new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((resolve,reject)=>{ws.addEventListener('open',resolve,{once:true});ws.addEventListener('error',reject,{once:true})});
  let id=0;const pending=new Map();
  ws.addEventListener('message',event=>{
    const msg=JSON.parse(event.data);if(!pending.has(msg.id))return;
    const {resolve,reject}=pending.get(msg.id);pending.delete(msg.id);
    if(msg.error)reject(new Error(msg.error.message));else resolve(msg.result);
  });
  function command(method,params={}){
    return new Promise((resolve,reject)=>{
      const next=++id;pending.set(next,{resolve,reject});
      ws.send(JSON.stringify({id:next,method,params}));
    });
  }
  const evaluate=async code=>{
    const result=await command('Runtime.evaluate',{expression:code,returnByValue:true,awaitPromise:true});
    if(result.exceptionDetails)throw new Error(result.exceptionDetails.text);
    return result.result?.value;
  };
  await command('Page.enable');await command('Runtime.enable');
  const navigation=await command('Page.navigate',{url:'http://127.0.0.1:'+server.address().port+'/index.html'});
  assert.ok(!navigation.errorText,navigation.errorText||'navigation');
  await until(async()=>evaluate("!!document.querySelector('#source')"),'editor load');
  await until(async()=>evaluate('window.__attoReady===true'),'module script ready');
  const run=async(src,expected)=>{
    await evaluate("(()=>{let e=document.querySelector('#source');e.value="+JSON.stringify(src)+";document.querySelector('#run').click()})()");
    let lastOutput='not yet available';
    const state=await until(async()=>{
      const value=await evaluate("document.querySelector('#result').textContent");
      lastOutput=value;
      return value?.includes(expected)?value:null;
    },()=>expected+' (last browser output: '+lastOutput+')');
    return state;
  };
  assert.match(await run('pub fnc main(): int { return 6 * 7; }','main() -> 42'),/42/);
  console.log('PASS browser: actual Rust compiler Wasm and guest Wasm produce 42');
  const linkedMtdSource = await readFile(new URL('./fixtures/pass/173-class-mtd-executable.atto', import.meta.url), 'utf8');
  assert.equal(await run(linkedMtdSource, 'main() -> 20'), 'main() -> 20');
  console.log('PASS browser: compiled native class mtd methods dispatch and return 20');
  const illegalMtd = 'define class C as int value: 1; end mtd (self: C) update() -> int { self.value = 2; return 2; } pub fnc main(): int { return 0; }';
  assert.match(await run(illegalMtd, 'E_OWNERSHIP'), /E_OWNERSHIP/);
  console.log('PASS browser: immutable class receiver mutation fails closed');
  // Exercise the exact lexically captured, pure-function sample from the
  // public playground through Chromium's real Wasm compiler and guest workers.
  const lexical = await readFile(new URL('./fixtures/pass/playground-lexical-scope-pure.atto', import.meta.url), 'utf8');
  assert.equal(await run(lexical, 'main() -> 0'), '42\n42\n2\n42\nmain() -> 0');
  console.log('PASS browser: lexical capture, scope shadowing and pure arithmetic (42/42/2/42)');
  const impure = await readFile(new URL('./fixtures/fail/pure-transitive-call.atto', import.meta.url), 'utf8');
  assert.match(await run(impure, 'E_PURE'), /E_PURE/);
  console.log('PASS browser: transitive impurity is rejected by the compiler');
  // Parity-inspired lexical tests: Java's closure lifetime is broader, but
  // this deliberately bounded Rust/Wasm profile must preserve lexical values.
  for (const [fixture, expected] of [
    ['pure-lexical-two-captures.atto', 'main() -> 42'],
    ['pure-lexical-bool-capture.atto', 'main() -> 42'],
    ['pure-mutual-recursion.atto', 'main() -> 42'],
    ['lexical-constructor-lambda-capture.atto', 'main() -> 42']
  ]) {
    const source = await readFile(new URL('./fixtures/pass/' + fixture, import.meta.url), 'utf8');
    assert.match(await run(source, expected), /^main\(\) -> 42$/);
  }
  console.log('PASS browser: immutable lexical captures and mutually recursive pure functions');
  for (const [fixture, expected] of [
    ['pure-transitive-dead-branch.atto', 'E_PURE'],
    ['lexical-out-of-scope-capture.atto', 'E_CAPTURE'],
    ['lexical-escaping-mutable-counter.atto', 'E_CAPTURE'],
    ['lexical-constructor-lambda-escape.atto', 'E_UNSUPPORTED']
  ]) {
    const source = await readFile(new URL('./fixtures/fail/' + fixture, import.meta.url), 'utf8');
    assert.match(await run(source, expected), new RegExp(expected));
  }
  console.log('PASS browser: effects and invalid captures cannot masquerade as pure or lexical');



  assert.match(await run('pub fnc main(): int {\n  val x = 7;\n  x = 8;\n  return x;\n}','E_IMMUTABLE'),/E_IMMUTABLE/);
  console.log('PASS browser: val reassignment fails closed');
  assert.match(await run('pub fnc main(): int { return 6 * 8; }','main() -> 48'),/48/);
  console.log('PASS browser: different source produces independent Wasm guest result 48');
  const fib='fnc fib(int n): int {\n  if n < 2 then\n    return n;\n  fi\n  return fib(n - 1) + fib(n - 2);\n}\npub fnc main(): int {\n  for (let i = 0; i < 5; i++) do\n    stdio.println(fib(i));\n  done\n  return fib(20);\n}';
  assert.match(await run(fib,'main() -> 6765'),/^0\n1\n1\n2\n3\nmain\(\) -> 6765$/);
  console.log('PASS browser: recursion, for loop and granted stdio imports');
  assert.match(await run('pub fnc main(): int {\n  stdio.println("hi");\n  val x: int = 9223372036854775807;\n  return x + 1;\n}','E_PANIC'),/hi\nE_PANIC: integer overflow/);
  console.log('PASS browser: checked int overflow panics with output preserved');
  // Real Chromium: compiler-produced Wasm guest tag catches only ordinary
  // Error(message), preserving all successful Result values and fatal traps.
  const callerTrap =
    'fnc checked_id(int id): Result<int,string> {\n' +
    '  if id < 0 then return Err("bad id"); fi\n' +
    '  return Ok(id);\n' +
    '}\n' +
    'pub fnc main(): int {\n' +
    '  val Option<Result<int,string>> x = trap checked_id(-1);\n' +
    '  match x over\n' +
    '    on Some(result) -> { match result over\n' +
    '      on Ok(n) -> { return n; }\n' +
    '      on Err(msg) -> { stdio.println(msg); return 7; }\n' +
    '    end }\n' +
    '    on None -> { return 0; }\n' +
    '  end\n' +
    '}';
  assert.match(await run(callerTrap, 'main() -> 7'), /^bad id\nmain\(\) -> 7$/);
  console.log('PASS browser: trap Some(Err) remains Result data');

  const ordinaryThrow =
    'fnc leaf(): int { throw Error("failure"); }\n' +
    'fnc wrapper(): int { return leaf(); }\n' +
    'pub fnc main(): int {\n' +
    '  match trap wrapper() over\n' +
    '    on Some(v) -> { return v; }\n' +
    '    on None -> { return 9; }\n' +
    '  end\n' +
    '}';
  assert.match(await run(ordinaryThrow, 'main() -> 9'), /main\(\) -> 9$/);
  console.log('PASS browser: private guest Error unwinds across calls to None');

  const trappedPanic =
    'pub fnc main(): int {\n' +
    '  val n: int = 9223372036854775807;\n' +
    '  match trap (n + 1) over\n' +
    '    on Some(v) -> { return v; }\n' +
    '    on None -> { return 0; }\n' +
    '  end\n' +
    '}';
  assert.match(await run(trappedPanic, 'E_PANIC'), /E_PANIC: integer overflow/);
  console.log('PASS browser: trap never catches checked arithmetic panic');

  assert.match(await run('pub fnc main(): int { return 6 * 7; }','main() -> 42'),/42/);
  console.log('PASS browser: error recovery restores valid execution');
  const values='fnc label(int n): string {\n  match n over\n    on 0 -> { return "zero"; }\n    on _ -> { return "many"; }\n  end\n}\npub fnc main(): int {\n  val f: float = 7.0 / 2;\n  stdio.println(f);\n  stdio.println(1e7);\n  stdio.println(label(0) + "/" + label(3));\n  stdio.println("  pad ".trim() eq "pad");\n  return 1;\n}';
  assert.match(await run(values,'main() -> 1'),/^3\.5\n1\.0E7\nzero\/many\ntrue\nmain\(\) -> 1$/);
  console.log('PASS browser: float (Java Double.toString), string values and match');
  // Real browser acceptance for the five async playground showcases. This
  // compiles the exact .atto fixture source inside a Chromium Web Worker using
  // the pinned Rust-to-Wasm compiler, then executes the resulting guest Wasm.
  // Do not substitute JS simulation or the Node-only conformance results.
  for (const [filename, expectedReturn, expectedStdout] of [
    ['200-play-suspends.atto', 42, '42\n'],
    ['201-play-tasks-async.atto', 42, '42\n'],
    ['202-play-async-iterators.atto', 60, '60\n'],
    ['203-play-channel-bounded.atto', 6, '6\n'],
    ['204-play-channel-unbounded.atto', 42, '42\n'],
    ['205-play-scoped-span.atto', 0, 'span retained over suspension\n'],
    ['206-play-nested-scoped-spans.atto', 0, 'both spans alive\n'],
  ]) {
    const source = await readFile(new URL('./fixtures/pass/' + filename, import.meta.url), 'utf8');
    const browserOutput = await run(source, 'main() -> ' + expectedReturn);
    assert.equal(browserOutput, expectedStdout + 'main() -> ' + expectedReturn,
      filename + ': exact stdout and returned value from a real Chromium guest');
    if (filename==='205-play-scoped-span.atto' || filename==='206-play-nested-scoped-spans.atto') {
      const n=filename.startsWith('205')?1:2;
      assert.deepEqual(await evaluate('window.__attoLastSpanCounts'), {opens:n,closes:n},
        filename + ': span closed exactly once after await inside real guest Wasm');
    }
    console.log('PASS browser: async showcase ' + filename + ' compiled and executed');
  }

  // A timeout must close the suspended guest's live span before trapping.
  // The counter is read from Wasm linear memory, not inferred from stdout.
  const timeoutGuest = 'import span from pkg "core.telemetry"; import sleep from pkg "std.time"; '+
    'pub suspends fnc main(): int { using span("timeout") as s { with timeout(1) { '+
    'await sleep(2); s.set_attribute("not", "reached"); } } return 0; }';
  assert.match(await run(timeoutGuest,'E_PANIC'), /E_PANIC/);
  assert.deepEqual(await evaluate('window.__attoLastSpanCounts'), {opens:1,closes:1},
    'timeout must trigger exactly-once span cleanup before Wasm guest trap');
  console.log('PASS browser: timeout unwinds live span in actual guest Wasm');

  // A removed operator must not be silently admitted by either frontend.
  assert.match(await run('pub suspends fnc main(): int { return 1 ^^ 2; }', 'E_UNSUPPORTED'),
    /E_UNSUPPORTED/);
  console.log('PASS browser: removed ^^ rejects in async source');

  // Actual .atto compiler -> guest Wasm, not a host-side graph simulation.
  // These are v2 LEGACY actor-only pipes; they do not establish typed
  // Channel/Stream/PipeHandle or authenticated TenantVM parity.
  for (const [fixture, spelling] of [
    ['207-v2-actor-pipe-forward.atto', 'pipe'],
    ['208-v2-actor-rpipe-reverse.atto', 'rpipe'],
  ]) {
    const guest = await readFile(new URL('./fixtures/pass/' + fixture, import.meta.url), 'utf8');
    const stdout = await run(guest, 'main() -> 42');
    assert.match(stdout, /^42\nmain\(\) -> 42$/,
      spelling + ' must deliver one ordered DATA item into compiled Atto receive');
    console.log('PASS browser: ' + spelling + ' actor guest DATA edge delivered 42');
  }
  for (const [fixture, expected] of [
    ['208-v2-actor-rpipe-direction-mix.atto', 'E_PIPE_DIRECTION'],
    ['209-v2-actor-pipe-missing-source.atto', 'E_TYPE'],
    ['210-v2-actor-rpipe-type-mismatch.atto', 'E_TYPE'],
  ]) {
    const rejected = await readFile(new URL('./fixtures/fail/' + fixture, import.meta.url), 'utf8');
    assert.match(await run(rejected, expected), new RegExp(expected));
    console.log('PASS browser: denied invalid actor graph ' + fixture);
  }

  // Compile real .atto source in the Rust-to-Wasm frontend, then execute
  // the compiled Wasm guest in Chromium. The capacity-1 channel must force
  // the actor emission to suspend and resume without dropping either item.
  // No JS transport simulation, sleep-based completion or host ActorRef.
  for (const filename of [
    '208-v2-actor-channel-actor-pipe.atto',
    '209-v2-actor-channel-actor-rpipe.atto',
    '217-v2-declared-port-pipe.atto',
    '218-v2-declared-port-rpipe.atto',
    // Typed checked Result<PipeHandle,PipeSetupError> executes in the same
    // *real compiled guest* path as the legacy fixtures, not a JS graph mock.
    '219-v2-checked-actor-channel-actor-pipe.atto',
    '220-v2-checked-actor-channel-actor-rpipe.atto',
    '221-v2-checked-setup-rollback.atto',
    '222-v2-checked-setup-output-occupied.atto',
    '222-v2-checked-output-occupied-rollback.atto',
    '223-v2-checked-input-occupied-rollback.atto',
    '224-v2-checked-channel-occupied-rollback.atto',
    // READY/DONE are distinct Futures, await spawn resumes, and checked
    // actor bang is exercised by the actual browser-compiled Wasm guest.
    '230-v2-actor-ready-future-pipe.atto',
    '231-v2-await-spawn-ready-pipe.atto',
    '232-v2-direct-actor-pipe-bang.atto',
    '233-v2-direct-actor-rpipe-bang.atto',
    '234-v2-direct-actor-bang-duplicate-denied.atto',
  ]) {
    const guest = await readFile(new URL('./fixtures/pass/' + filename, import.meta.url), 'utf8');
    const observed = await run(guest, 'main() -> 42');
    assert.equal(observed, '40\n2\nmain() -> 42',
      filename + ': exact ordered DATA and BOTH completion futures in real Wasm');
    assert.equal(await evaluate('window.__attoLastPanicCode'), null,
      filename + ': guest must complete without a hidden runtime trap');
    console.log('PASS browser: real bounded actor-channel-actor guest ' + filename);
  }
  for (const filename of [
    '210-v2-pipe-double-attach.atto',
    '211-v2-pipe-rendezvous-denial.atto',
    '212-v2-pipe-duplicate-consumer.atto',
    '213-v2-pipe-stopped-producer.atto',
    '214-v2-pipe-closed-channel.atto',
    '215-v2-pipe-stopped-consumer.atto',
    '216-v2-pipe-stopped-actor-sink.atto',
    '225-v2-pipe-claimed-channel-send.atto',
    '226-v2-pipe-claimed-channel-receive.atto',
    '227-v2-pipe-claimed-channel-close.atto',
    '228-v2-pipe-claimed-for-await.atto',
    '229-v2-pipe-claimed-rx-poll.atto',
  ]) {
    const guest = await readFile(new URL('./fixtures/panic/' + filename, import.meta.url), 'utf8');
    assert.match(await run(guest, 'E_PANIC'), /^E_PANIC:/);
    assert.equal(await evaluate('window.__attoLastPanicCode'), 10,
      filename + ': invalid graph rejected before any unsafe edge publication');
    console.log('PASS browser: real guest rejects unsafe pipe ' + filename);
  }

  // Byte-identical canonical atto-platform-conformance fixture #102:
  // both spellings attempt producer -> consumer, not an implicit duplex.
  const canonicalDuplicate = await readFile(new URL(
    './fixtures/conformance/pipe-rpipe-v1/actor-duplicate-direction.atto',
    import.meta.url), 'utf8');
  assert.match(await run(canonicalDuplicate, 'E_PANIC'), /^E_PANIC:/);
  assert.equal(await evaluate('window.__attoLastPanicCode'), 10,
    'canonical duplicate lexical direction must trap during guest DATA admission');

  // Real compiler -> Wasm guest: a second same-direction actor connection
  // must trap before publishing any cycle (not hang the actor-link walk).
  // The v2 actor-only runtime remains experimental and lacks checked handles.
  const duplicatePipeGuest = await readFile(new URL(
    './fixtures/panic/207-v2-duplicate-actor-pipe.atto', import.meta.url), 'utf8');
  assert.match(await run(duplicatePipeGuest, 'E_PANIC'), /^E_PANIC:/);
  assert.equal(await evaluate('window.__attoLastPanicCode'), 10,
    'duplicate actor pipe must use bounded graph-admission trap code 10');
  console.log('PASS browser: actual guest traps duplicate actor pipe before cycle');
  assert.match(await run('pub fnc main(): int { return 42; }', 'main() -> 42'),
    /^main\(\) -> 42$/);
  console.log('PASS browser: fresh guest remains runnable after rejected actor pipe');


  // The full source bytes are mirrored from atto-lang.github.io PR #153 (comment-only header refresh of PR #127),
  // not a hand-written approximation. Test a real Chromium dedicated Worker,
  // with the actual Rust-compiled wasm32 compiler and its emitted guest Wasm.
  const stateSource = await readFile(new URL('./browser/fixtures/play-state-containers.atto',import.meta.url),'utf8');
  const blobHash = createHash('sha1')
    .update('blob '+Buffer.byteLength(stateSource)).update(Buffer.from([0])).update(stateSource).digest('hex');
  assert.equal(blobHash, '76f853511691aed4233209cb551571ab040983e2',
    'exact /play preview Git blob must match website PR, not a divergent test sample');
  assert.equal(await run(stateSource, 'main() -> 0'), '2\n0\nmain() -> 0');
  console.log('PASS browser: byte-verified /play state preview compiles AND executes in Chromium Worker');

  // Exact website preview blob (MANIFEST.tsv): defer/recover/try/raise in a
  // real Worker, through the wasm32 compiler and legacy Wasm EH guest tags.
  const unwindSource = await readFile(new URL('./playground_previews/defer-recover.atto', import.meta.url), 'utf8');
  const unwindBlob = createHash('sha1')
    .update('blob ' + Buffer.byteLength(unwindSource)).update(Buffer.from([0])).update(unwindSource).digest('hex');
  assert.equal(unwindBlob, '5395c8159b1f7d111ea80d22ea6590960f340c1e',
    'exact /play defer-recover preview Git blob must match the website');
  assert.equal(await run(unwindSource, 'main() -> 63'),
    'raise finally\nraise cleanup A\nraise cleanup B\nrecovering raise\nthrow cleanup\n' +
    'recovering unhandled throw\ncaught finally\ncaught cleanup\nmain() -> 63');
  console.log('PASS browser: byte-verified /play defer-recover preview executes in Chromium Worker');

  // Real guest fault while an exclusive state ticket is physically active.
  // The Worker aborts only after unwind, and each new Worker has fresh tickets.
  const stateOverflow = stateSource.replace('counter.increment(2);',
    'counter.increment(9223372036854775807);\n  counter.increment(1);');
  assert.notEqual(stateOverflow,stateSource,'overflow fixture must alter the program');
  assert.match(await run(stateOverflow,'E_PANIC'),/E_PANIC: integer overflow/);
  assert.equal(await run(stateSource, 'main() -> 0'), '2\n0\nmain() -> 0');
  console.log('PASS browser: state write panic aborts host ticket and next Worker recovers');

  const independent = await readFile(new URL('./fixtures/pass/180-standalone-state-critical.atto',import.meta.url),'utf8');
  assert.equal(await run(independent,'main() -> 0'), '2\n0\n7\n2\nmain() -> 0');
  console.log('PASS browser: separate state instances, critical read/write, reset');

  const unguarded = await readFile(new URL('./fixtures/fail/196-state-same-class-access.atto',import.meta.url),'utf8');
  assert.match(await run(unguarded,'E_STATE_BORROW'), /^E_STATE_BORROW:/);
  const spacedCriticalRead = await readFile(new URL('./fixtures/fail/188-state-spaced-critical-read.atto',import.meta.url),'utf8');
  assert.match(await run(spacedCriticalRead,'E_SYNTAX'), /^E_SYNTAX:/);
  assert.equal(await run(stateSource,'main() -> 0'),'2\n0\nmain() -> 0',
    'invalid code must not poison the next Worker or leave stale guest bytes');
  console.log('PASS browser: state capability bypass and malformed keyword fail closed; recovery executes');

  assert.ok(requests.some(x=>x.path==='/state-runtime.mjs'),
    'the browser Worker must load the protected-state host admission module');
  assert.ok(requests.some(x=>x.path==='/atto_compiler.wasm'));
  assert.ok(requests.every(x=>x.method==='GET'));
  console.log('PASS browser: no source POST or server-side compilation');
}finally{
  try{ws?.close()}catch{}
  if(chrome?.exitCode===null){chrome.kill('SIGKILL');await sleep(200)}
  await new Promise(resolve=>server.close(resolve));
  await rm(tmp,{recursive:true,force:true,maxRetries:5,retryDelay:50});
}
