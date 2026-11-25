// Actual Chromium + web workers + Rust-compiled compiler + guest Wasm.
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {spawn} from 'node:child_process';
import {mkdtemp,readFile,rm} from 'node:fs/promises';
import {join} from 'node:path';
import {tmpdir} from 'node:os';
const root=new URL('.',import.meta.url), tmp=await mkdtemp(join(tmpdir(),'atto-rust-browser-'));
const requests=[];
const files=new Map([
  ['/index.html',[new URL('./browser/index.html',import.meta.url),'text/html; charset=utf-8']],
  ['/worker.mjs',[new URL('./browser/worker.mjs',import.meta.url),'text/javascript; charset=utf-8']],
  ['/run-watchdog.mjs',[new URL('./browser/run-watchdog.mjs',import.meta.url),'text/javascript; charset=utf-8']],
  ['/actor-lifecycle.mjs',[new URL('./browser/actor-lifecycle.mjs',import.meta.url),'text/javascript; charset=utf-8']],
  ['/actor-import-contract.mjs',[new URL('./browser/actor-import-contract.mjs',import.meta.url),'text/javascript; charset=utf-8']],
  ['/actor-worker.mjs',[new URL('./browser/actor-worker.mjs',import.meta.url),'text/javascript; charset=utf-8']],
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
  throw new Error(label+' timed out: '+last);
}
let chrome,ws;
try{
  await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
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
    if(result.exceptionDetails)throw new Error(
      result.exceptionDetails.exception?.description || result.exceptionDetails.text);
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
    },expected+' (last browser output: '+lastOutput+')');
    return state;
  };
  assert.match(await run('pub fnc main(): int { return 6 * 7; }','main() -> 42'),/42/);
  console.log('PASS browser: actual Rust compiler Wasm and guest Wasm produce 42');

  const classExample = [
    'define class Point as',
    '  int x: 40',
    '  int y: 33',
    'end',
    'pub fnc main(): int {',
    '  val p = new Point();',
    '  return p.x + p.y;',
    '}',
  ].join('\n');
  assert.match(await run(classExample,'main() -> 73'),/73/);
  console.log('PASS browser: native Rust-compiled Wasm compiler generates and runs a class allocation guest');
  assert.match(await run(classExample.replace('new Point()', 'Point()'),'E_NEW_REQUIRED'), /E_NEW_REQUIRED/);
  console.log('PASS browser: bare class calls are diagnosed without guest execution');
  assert.match(await run(classExample,'main() -> 73'),/73/);
  console.log('PASS browser: explicit new recovers after rejected bare class invocation');
  assert.match(await run('pub fnc main(): int {\n  val x = 7;\n  x = 8;\n  return x;\n}','E_IMMUTABLE'),/E_IMMUTABLE/);
  console.log('PASS browser: val reassignment fails closed');
  assert.match(await run('pub fnc main(): int { return 6 * 8; }','main() -> 48'),/48/);
  console.log('PASS browser: different source produces independent Wasm guest result 48');
  const fib='fnc fib(int n): int {\n  if n < 2 then\n    return n;\n  fi\n  return fib(n - 1) + fib(n - 2);\n}\npub fnc main(): int {\n  for (let i = 0; i < 5; i++) do\n    stdio.println(fib(i));\n  done\n  return fib(20);\n}';
  assert.match(await run(fib,'main() -> 6765'),/^0\n1\n1\n2\n3\nmain\(\) -> 6765$/);
  console.log('PASS browser: recursion, for loop and granted stdio imports');
  assert.match(await run('pub fnc main(): int {\n  stdio.println("hi");\n  val x: int = 9223372036854775807;\n  return x + 1;\n}','E_PANIC'),/hi\nE_PANIC: integer overflow/);
  console.log('PASS browser: checked int overflow panics with output preserved');
  assert.match(await run('pub fnc main(): int { return 6 * 7; }','main() -> 42'),/42/);
  console.log('PASS browser: error recovery restores valid execution');
  const actorExample=[
    'define actor Worker as',
    '  receive(msg: int) -> int { return msg + 1; }',
    'end',
    'pub fnc main(): int {',
    '  val worker: Worker = spawn Worker();',
    '  return 0;',
    '}',
  ].join('\n');
  await evaluate("window.__attoActorMessages=[{handle:1,value:'41'}]");
  assert.match(await run(actorExample,'actor[1] receive(41) -> 42'),/actor\[1\] receive\(41\) -> 42/);
  await evaluate("window.__attoActorMessages=undefined");
  console.log('PASS browser: guest spawn creates real nested Web Worker with separate Wasm instance and typed i64 reply');
  assert.ok(requests.some(x=>x.path==='/actor-worker.mjs'),'actor worker was actually loaded');
  // Re-use a single actor's mailbox sequentially; never rely on another
  // 'ready' signal after the first typed response.
  await evaluate("window.__attoActorMessages=[{handle:1,value:'41'},{handle:1,value:'42'}]");
  assert.match(await run(actorExample,'actor[1] receive(42) -> 43'),/actor\[1\] receive\(41\) -> 42\nactor\[1\] receive\(42\) -> 43/);
  console.log('PASS browser: sequential mailbox messages retain ordering');
  // Two spawns get different worker handles and independent Wasm instances.
  const twoActors=actorExample.replace('  return 0;', '  val other: Worker = spawn Worker();\n  return 0;');
  await evaluate("window.__attoActorMessages=[{handle:1,value:'41'},{handle:2,value:'42'}]");
  assert.match(await run(twoActors,'actor[2] receive(42) -> 43'),/actor\[1\] receive\(41\) -> 42\nactor\[2\] receive\(42\) -> 43/);
  console.log('PASS browser: two real actor workers have independent handles and execution instances');
  await evaluate("window.__attoActorMessages=[{handle:1,value:'9223372036854775808'}]");
  assert.match(await run(actorExample,'E_MESSAGE_RANGE'),/E_MESSAGE_RANGE/);
  await evaluate("window.__attoActorMessages=undefined");
  console.log('PASS browser: out-of-range message is rejected before worker invocation');
  await evaluate("window.__attoActorMessages=[{handle:1,value:{untrustedPointer:123}}]");
  assert.match(await run(actorExample,'E_MESSAGE_TYPE'),/E_MESSAGE_TYPE/);
  await evaluate("window.__attoActorMessages=undefined");
  console.log('PASS browser: object and pointer-shaped messages fail closed');
  // Source-level send() compiles to a checked Result-valued mailbox admission.
  // Responses are delivered later by the host from the real dedicated actor.
  const sourceSend=[
    'define actor Worker as receive(msg: int) -> int { return msg + 1; } end',
    'pub fnc main(): int {',
    '  val worker = spawn Worker();',
    '  val accepted: Result<void, SendError> = worker.send(41);',
    '  if accepted.is_ok() then return 42; else return 0; fi',
    '}',
  ].join('\n');
  assert.match(await run(sourceSend,'actor[1] receive(41) -> 42'),
    /^actor\[1\] receive\(41\) -> 42\nmain\(\) -> 42$/);
  console.log('PASS browser: source send Result admits real scalar message and returns before completion');
  const sourceTwoSends=sourceSend.replace(
    '  if accepted.is_ok() then return 42; else return 0; fi',
    '  val second = worker.send(42); if accepted.is_ok() , second.is_ok() then return 42; else return 0; fi');
  assert.match(await run(sourceTwoSends,'actor[1] receive(42) -> 43'),
    /^actor\[1\] receive\(41\) -> 42\nactor\[1\] receive\(42\) -> 43\nmain\(\) -> 42$/);
  console.log('PASS browser: source sends preserve per-actor FIFO across real Worker messages');

  // Admission is bounded at 32 pending messages, before any asynchronous
  // receive completes. The 33rd send must produce Result::Err while all
  // admitted messages are delivered in deterministic per-actor order.
  const fullSource=[
    'define actor Worker as receive(msg: int) -> int { return msg + 1; } end',
    'pub fnc main(): int {',
    '  val worker = spawn Worker();',
    '  let rejected = 0;',
    '  for (let i = 0; i < 33; i++) do',
    '    val attempt = worker.send(i);',
    '    if attempt.is_err() then rejected = rejected + 1; fi',
    '  done',
    '  return rejected;',
    '}',
  ].join('\n');
  const fullOutput=await run(fullSource,'main() -> 1');
  assert.equal((fullOutput.match(/receive\(/g)||[]).length,32,
    'exactly 32 messages admitted and executed');
  assert.ok(!fullOutput.includes('receive(32)'), '33rd message must not execute');
  assert.match(fullOutput,/actor\[1\] receive\(31\) -> 32\nmain\(\) -> 1$/);
  console.log('PASS browser: real 32-slot admission backpressure and 33rd Result::Err');


  assert.match(await run(sourceSend.replace('worker.send(41)','worker.get_done()'), 'E_UNSUPPORTED'),
    /E_UNSUPPORTED/);
  console.log('PASS browser: unawaited get_done task values remain unsupported');

  // True async source scope join: main() has no continuation after await,
  // and the parent Worker cannot report success before the child acknowledges
  // all accepted messages and explicit terminal shutdown.
  const sourceAwait=[
    'define actor Worker as receive(msg: int) -> int { return msg + 1; } end',
    'pub fnc main(): void {',
    '  val worker = spawn Worker();',
    '  val accepted = worker.send(41);',
    '  await worker.get_done();',
    '}',
  ].join('\n');
  assert.equal(await run(sourceAwait,'main() -> 0'),
    'actor[1] receive(41) -> 42\nmain() -> 0');
  console.log('PASS browser: terminal source await joins acknowledged actor shutdown asynchronously');
  assert.ok(requests.some(x=>x.path==='/actor-lifecycle.mjs'),
    'real lifecycle module fetched by browser Worker');
  const panicAwait=sourceAwait.replace('msg + 1','msg + 9223372036854775807');
  const panicResult=await run(panicAwait,'E_ACTOR_PANIC_1');
  assert.match(panicResult,/^E_HOST: E_ACTOR_PANIC_1$/);
  assert.ok(!panicResult.includes('main() -> 0'),
    'panicked actor must not produce successful completed task');
  console.log('PASS browser: actor panic rejects terminal join, never invents completion');
  const illegalAwait=sourceAwait.replace('  await worker.get_done();',
    '  await worker.get_done(); stdio.println(99);');
  assert.match(await run(illegalAwait,'E_UNSUPPORTED'),/E_UNSUPPORTED/);
  console.log('PASS browser: suspension with arbitrary continuation fails closed');


  // Stage-3+ true browser regressions: compiler-generated messages must round
  // trip signed i64 boundaries with sequence-correlated worker replies.
  const signedBoundaries=[
    'define actor Worker as receive(msg: int) -> int { return msg; } end',
    'pub fnc main(): void {',
    '  val worker = spawn Worker();',
    '  val lower = worker.send(-9223372036854775808);',
    '  val upper = worker.send(9223372036854775807);',
    '  await worker.get_done();',
    '}',
  ].join('\n');
  assert.equal(await run(signedBoundaries,'main() -> 0'),
    'actor[1] receive(-9223372036854775808) -> -9223372036854775808\n'+
    'actor[1] receive(9223372036854775807) -> 9223372036854775807\n'+
    'main() -> 0');
  console.log('PASS browser: signed i64 min and max survive actual actor Web Worker transport');

  const emptyJoin=[
    'define actor Worker as receive(msg: int) -> int { return msg; } end',
    'pub fnc main(): void {',
    '  val worker = spawn Worker();',
    '  await worker.get_done();',
    '}',
  ].join('\n');
  assert.equal(await run(emptyJoin,'main() -> 0'),'main() -> 0');
  console.log('PASS browser: zero-message actor completes only after explicit child shutdown');

  const twoActorJoin=[
    'define actor Worker as receive(msg: int) -> int { return msg + 1; } end',
    'pub fnc main(): void {',
    '  val first = spawn Worker();',
    '  val second = spawn Worker();',
    '  val a = first.send(40);',
    '  val b = second.send(50);',
    '  await second.get_done();',
    '}',
  ].join('\n');
  assert.equal(await run(twoActorJoin,'main() -> 0'),
    'actor[1] receive(40) -> 41\nactor[2] receive(50) -> 51\nmain() -> 0');
  console.log('PASS browser: separate actor joins preserve independent FIFO execution');

  const fourActors=[
    'define actor Worker as receive(msg: int) -> int { return msg + 1; } end',
    'pub fnc main(): void {',
    '  val first = spawn Worker();',
    '  val second = spawn Worker();',
    '  val third = spawn Worker();',
    '  val fourth = spawn Worker();',
    '  val a = first.send(1);',
    '  val b = second.send(2);',
    '  val c = third.send(3);',
    '  val d = fourth.send(4);',
    '  await fourth.get_done();',
    '}',
  ].join('\n');
  assert.equal(await run(fourActors,'main() -> 0'),
    'actor[1] receive(1) -> 2\nactor[2] receive(2) -> 3\n'+
    'actor[3] receive(3) -> 4\nactor[4] receive(4) -> 5\nmain() -> 0');
  console.log('PASS browser: four real actor Workers terminate independently within host quota');

  const panicAfterReply=[
    'define actor Worker as',
    '  receive(msg: int) -> int {',
    '    if msg eq 0 then return msg; fi',
    '    return msg + 9223372036854775807;',
    '  }',
    'end',
    'pub fnc main(): void {',
    '  val worker = spawn Worker();',
    '  val first = worker.send(0);',
    '  val second = worker.send(1);',
    '  await worker.get_done();',
    '}',
  ].join('\n');
  const panicAfterReplyOutput=await run(panicAfterReply,'E_ACTOR_PANIC_1');
  assert.match(panicAfterReplyOutput,/^E_HOST: E_ACTOR_PANIC_1$/);
  assert.ok(!panicAfterReplyOutput.includes('main() ->'),
    'one successful reply cannot certify later panicked shutdown');
  console.log('PASS browser: successful first reply cannot mask later actor panic');

  const fullAwait=[
    'define actor Worker as receive(msg: int) -> int { return msg + 1; } end',
    'pub fnc main(): void {',
    '  val worker = spawn Worker();',
    '  for (let i = 0; i < 33; i++) do',
    '    val attempt = worker.send(i);',
    '  done',
    '  await worker.get_done();',
    '}',
  ].join('\n');
  const fullAwaitOutput=await run(fullAwait,'main() -> 0');
  assert.equal((fullAwaitOutput.match(/receive\(/g)||[]).length,32);
  assert.ok(!fullAwaitOutput.includes('receive(32)'), '33rd payload cannot bypass admission');
  assert.match(fullAwaitOutput,/actor\[1\] receive\(31\) -> 32\nmain\(\) -> 0$/);
  console.log('PASS browser: terminal join drains exactly 32 admitted messages after mailbox-full');

  // The browser must never run an arbitrary post-suspension continuation
  // without a compiled resumable frame. Assert recovery after failure.
  assert.match(await run(
    sourceAwait.replace('  await worker.get_done();',
      '  await worker.get_done(); return;'), 'E_UNSUPPORTED'),/E_UNSUPPORTED/);
  assert.equal(await run(emptyJoin,'main() -> 0'),'main() -> 0');
  console.log('PASS browser: forbidden continuation rejects without poisoning next actor run');

  // Directly test the real child worker's protocol gate. These are NOT fake
  // worker replies: a real browser-generated guest Wasm module is instantiated
  // inside a dedicated Chromium Web Worker; hostile host messages are injected.
  const protocolProbe = await evaluate(`(async()=>{
    const raw=await (await fetch('/atto_compiler.wasm')).arrayBuffer();
    const api=new WebAssembly.Instance(new WebAssembly.Module(raw),{}).exports;
    const code=new TextEncoder().encode(
      'define actor Worker as receive(msg: int) -> int { return msg + 1; } end '+
      'pub fnc main(): int { return 0; }');
    if(code.length>api.input_capacity())throw new Error('compiler capacity');
    new Uint8Array(api.memory.buffer,api.input_ptr(),code.length).set(code);
    if(api.compile(code.length)!==0)throw new Error('direct probe compile failed');
    const guest=new Uint8Array(api.memory.buffer,api.result_ptr(),api.result_length()).slice();
    if(!WebAssembly.validate(guest))throw new Error('invalid guest in direct probe');
    async function probe(kind){
      const worker=new Worker('/actor-worker.mjs',{type:'module'});
      const next=()=>new Promise((resolve,reject)=>{
        const timer=setTimeout(()=>reject(new Error('probe worker timeout')),2000);
        worker.onmessage=({data})=>{clearTimeout(timer);resolve(data)};
        worker.onerror=()=>{clearTimeout(timer);reject(new Error('probe worker error'))};
      });
      try{
        let response=next();
        worker.postMessage({kind:'init',actorId:0,wasm:guest});
        if((await response).kind!=='ready')throw new Error('probe init');
        response=next();
        if(kind==='future')worker.postMessage({kind:'message',seq:1,value:41n});
        if(kind==='missing')worker.postMessage({kind:'message',value:41n});
        if(kind==='invalid')worker.postMessage({kind:'message',seq:0,value:{pointer:42}});
        if(kind==='replay'){
          worker.postMessage({kind:'message',seq:0,value:41n});
          const reply=await response;
          if(reply.kind!=='reply'||reply.seq!==0||reply.value!==42n)
            throw new Error('probe first reply not correlated');
          response=next();
          worker.postMessage({kind:'message',seq:0,value:42n});
        }
        const outcome=await response;
        if(outcome.kind!=='error')throw new Error('hostile actor message accepted: '+outcome.kind);
        return outcome.message;
      }finally{worker.terminate()}
    }
    return {
      future:await probe('future'),
      replay:await probe('replay'),
      missing:await probe('missing'),
      invalid:await probe('invalid'),
    };
  })()`);
  assert.deepEqual(protocolProbe,{
    future:'E_ACTOR_MESSAGE_SEQ',
    replay:'E_ACTOR_MESSAGE_SEQ',
    missing:'E_ACTOR_MESSAGE_SEQ',
    invalid:'E_MESSAGE_TYPE',
  });
  console.log('PASS browser: real actor Worker rejects future sequence IDs');
  console.log('PASS browser: real actor Worker rejects replayed sequence IDs');
  console.log('PASS browser: real actor Worker rejects missing sequence IDs');
  console.log('PASS browser: real actor Worker rejects object/pointer-shaped messages');

  // Exercise the *real child Worker*, not just a unit-test import checker:
  // crafted valid Wasm modules with duplicate/nonfunction/foreign imports
  // must be refused before receiving any actor message.
  const guestImportAttacks=await evaluate(`(async()=>{
    const utf8=new TextEncoder();
    const leb=n=>{const out=[];do{let b=n&127;n=Math.floor(n/128);out.push(n?b|128:b)}while(n);return out};
    const name=s=>{const bytes=[...utf8.encode(s)];return [...leb(bytes.length),...bytes]};
    const sec=(id,v)=>[id,...leb(v.length),...v];
    function guest(specs){
      let imports=[], types=[];
      const signatures={
        actor_send:{params:[127,126],results:[127]},
        actor_join:{params:[127],results:[]},
        actor_spawn:{params:[127],results:[127]},
        print_i64:{params:[126],results:[]},
        print_bool:{params:[127],results:[]},
        print_str:{params:[127,127],results:[]},
        print_newline:{params:[],results:[]},
      };
      for(let i=0;i<specs.length;i++){
        const spec=specs[i], baseline=signatures[spec.field]||signatures.actor_send;
        const params=spec.params||baseline.params, results=spec.results||baseline.results;
        types.push(96,...leb(params.length),...params,...leb(results.length),...results);
        imports.push(...name(spec.module||'atto'),...name(spec.field||'actor_send'));
        if(spec.kind==='global')imports.push(3,127,0);
        else imports.push(0,...leb(i));
      }
      const bytes=new Uint8Array([
        0,97,115,109,1,0,0,0,
        ...sec(1,[...leb(specs.length),...types]),
        ...sec(2,[...leb(specs.length),...imports])
      ]);
      if(!WebAssembly.validate(bytes))throw new Error('invalid test Wasm fixture');
      return bytes;
    }
    async function invalid_import(specs){
      const worker=new Worker('/actor-worker.mjs',{type:'module'});
      try{
        const received=new Promise((resolve,reject)=>{
          const timeout=setTimeout(()=>reject(new Error('import probe timed out')),2000);
          worker.onmessage=({data})=>{clearTimeout(timeout);resolve(data)};
          worker.onerror=()=>{clearTimeout(timeout);reject(new Error('actor init failed'))};
        });
        worker.postMessage({kind:'init',actorId:0,wasm:guest(specs)});
        const message=await received;
        if(message.kind!=='error')throw new Error('bad import unexpectedly admitted');
        return message.message;
      }finally{worker.terminate()}
    }
    return {
      duplicate:await invalid_import([{field:'actor_send'},{field:'actor_send'}]),
      wrong_kind:await invalid_import([{field:'actor_send',kind:'global'}]),
      foreign_module:await invalid_import([{module:'env',field:'actor_send'}]),
      send_wrong_param:await invalid_import([{field:'actor_send',params:[126]}]),
      send_wrong_order:await invalid_import([{field:'actor_send',params:[126,127]}]),
      join_wrong_result:await invalid_import([{field:'actor_join',results:[127]}]),
      print_wrong_param:await invalid_import([{field:'print_bool',params:[126]}]),
    };
  })()`);
  assert.deepEqual(guestImportAttacks,{
    duplicate:'E_ACTOR_IMPORT_DUPLICATE',
    wrong_kind:'E_ACTOR_IMPORT_KIND',
    foreign_module:'E_ACTOR_IMPORT_NAME',
    send_wrong_param:'E_ACTOR_IMPORT_SIGNATURE',
    send_wrong_order:'E_ACTOR_IMPORT_SIGNATURE',
    join_wrong_result:'E_ACTOR_IMPORT_SIGNATURE',
    print_wrong_param:'E_ACTOR_IMPORT_SIGNATURE',
  });
  console.log('PASS browser: child rejects duplicate actor capability imports');
  console.log('PASS browser: child rejects wrong-kind global imports');
  console.log('PASS browser: child rejects foreign namespace capability imports');
  console.log('PASS browser: child rejects mismatched actor_send parameter count');
  console.log('PASS browser: child rejects swapped actor_send parameter types');
  console.log('PASS browser: child rejects actor_join returning a value');
  console.log('PASS browser: child rejects print_bool accepting i64');


  // A runaway main() executes synchronously inside a dedicated compiler
  // worker. The independent page timer MUST remain responsive and be the
  // only finalizer; no late worker result may replace its terminal timeout.
  const runawayMain='pub fnc main(): int { loop { } return 0; }';
  await evaluate("(()=>{let e=document.querySelector('#source');e.value="+JSON.stringify(runawayMain)+";document.querySelector('#run').click()})()");
  await sleep(150);
  assert.equal(await evaluate("(()=>{document.title='parent-alive';return document.title})()"),
    'parent-alive','runaway guest cannot freeze parent browser event loop');
  const watchdogResult=await until(async()=>{
    const result=await evaluate("document.querySelector('#result').textContent");
    return result==='E_TIMEOUT'?result:null;
  },'real compiler-worker infinite main watchdog');
  assert.equal(watchdogResult,'E_TIMEOUT');
  assert.equal(await evaluate("document.querySelector('#run').disabled"),false);
  assert.equal(await run('pub fnc main(): int { return 6 * 7; }','main() -> 42'),'main() -> 42');
  console.log('PASS browser: infinite synchronous main is terminated by independent page watchdog');
  console.log('PASS browser: watchdog termination releases button and next guest executes');

  const runawayActor=[
    'define actor Worker as',
    '  receive(msg: int) -> int { loop { } return msg; }',
    'end',
    'pub fnc main(): void {',
    '  val worker = spawn Worker();',
    '  val accepted = worker.send(41);',
    '  await worker.get_done();',
    '}',
  ].join('\n');
  assert.equal(await run(runawayActor,'E_ACTOR_TIMEOUT'),'E_HOST: E_ACTOR_TIMEOUT');
  assert.equal(await run('pub fnc main(): int { return 43; }','main() -> 43'),'main() -> 43');
  console.log('PASS browser: runaway receive terminates child actor without false get_done success');
  console.log('PASS browser: child timeout does not poison subsequent compiler/actor runs');

  const excessiveOutput=[
    'pub fnc main(): int {',
    '  for (let i = 0; i < 6000; i++) do',
    '    stdio.println(i);',
    '  done',
    '  return 42;',
    '}',
  ].join('\n');
  assert.equal(await run(excessiveOutput,'E_OUTPUT_LIMIT'),'E_OUTPUT_LIMIT');
  assert.equal(await run('pub fnc main(): int { return 44; }','main() -> 44'),'main() -> 44');
  console.log('PASS browser: guest output quota rejects excessive stdout without success');
  console.log('PASS browser: output quota failure preserves next-run isolation');

  await evaluate("window.__attoActorMessages=[{handle:1,value:'9'.repeat(1000)}]");
  const largeHostPayload=await run(actorExample,'E_MESSAGE_TYPE');
  assert.match(largeHostPayload,/E_MESSAGE_TYPE/);
  await evaluate("window.__attoActorMessages=undefined");
  console.log('PASS browser: oversized external scalar string rejected before BigInt conversion');

  // Guest Wasm memory emitted by the compiler has min=max=1 page; growing it
  // would bypass the guest-private heap quota and must be impossible.
  const memoryCheck=await evaluate(`(async()=>{
    const raw=await (await fetch('/atto_compiler.wasm')).arrayBuffer();
    const api=new WebAssembly.Instance(new WebAssembly.Module(raw),{}).exports;
    const source='define class Unit as int value: 1 end '+
      'pub fnc main(): int { val item = new Unit(); return item.value; }';
    const bytes=new TextEncoder().encode(source);
    new Uint8Array(api.memory.buffer,api.input_ptr(),bytes.length).set(bytes);
    if(api.compile(bytes.length)!==0)throw new Error('memory probe compile failed');
    const wasm=new Uint8Array(api.memory.buffer,api.result_ptr(),api.result_length()).slice();
    const guest=new WebAssembly.Instance(new WebAssembly.Module(wasm),{}).exports;
    if(guest.memory.buffer.byteLength!==65536)throw new Error('incorrect initial memory');
    try{guest.memory.grow(1);return 'MEMORY_GROW_UNEXPECTEDLY_SUCCEEDED'}
    catch(e){return e instanceof RangeError?'MEMORY_GROW_DENIED':e.name}
  })()`);
  assert.equal(memoryCheck,'MEMORY_GROW_DENIED');
  console.log('PASS browser: compiler guest memory is fixed one-page and cannot grow');

  assert.ok(requests.some(x=>x.path==='/atto_compiler.wasm'));
  assert.ok(requests.every(x=>x.method==='GET'));
  console.log('PASS browser: no source POST or server-side compilation');
}finally{
  try{ws?.close()}catch{}
  if(chrome?.exitCode===null){chrome.kill('SIGKILL');await sleep(200)}
  await new Promise(resolve=>server.close(resolve));
  await rm(tmp,{recursive:true,force:true,maxRetries:5,retryDelay:50});
}
