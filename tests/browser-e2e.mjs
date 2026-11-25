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
  ['/actor-worker.mjs',[new URL('./browser/actor-worker.mjs',import.meta.url),'text/javascript; charset=utf-8']],
  ['/actor-lifecycle.mjs',[new URL('./browser/actor-lifecycle.mjs',import.meta.url),'text/javascript; charset=utf-8']],
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

  const senderProgram=[
    'define actor Worker as',
    '  receive(msg: int) -> int { return msg + 1; }',
    'end',
    'pub fnc main(): int {',
    '  val w: Worker = spawn Worker();',
    '  val first: Result<void, SendError> = w.send(41);',
    '  val second = w.send(42);',
    '  val full = w.send(43);',
    '  if first.is_ok() , second.is_ok() , full.is_full() then',
    '    return 42;',
    '  else',
    '    return 0;',
    '  fi',
    '}',
  ].join('\n');
  assert.match(await run(senderProgram,'main() -> 42'),/actor\[1\] receive\(41\) -> 42\nactor\[1\] receive\(42\) -> 43\nmain\(\) -> 42/);
  console.log('PASS browser: language send Result admits only two FIFO messages, rejects full third');
  const finished=await evaluate('window.__attoLastResult.actorCompletions');
  assert.equal(finished.length,1,'one terminal event for one actor');
  assert.deepEqual(finished.map(x=>[x.handle,x.state,x.admitted,x.delivered,x.dropped]),
    [[1,'FINISHED',2,2,0]]);
  console.log('PASS browser: FINISHED occurs only after two admitted handlers and shutdown ACK');
  const closeProgram=senderProgram.replace(
    '  val second = w.send(42);\n  val full = w.send(43);',
    '  val closed = w.close();\n  val second = w.send(42);\n  val full = w.send(43);'
  ).replace('first.is_ok() , second.is_ok() , full.is_full()',
    'first.is_ok() , closed , second.is_closed() , full.is_closed()');
  assert.match(await run(closeProgram,'main() -> 42'),/actor\[1\] receive\(41\) -> 42\nmain\(\) -> 42/);
  console.log('PASS browser: close blocks further admission but drains already admitted message');
  const closed=await evaluate('window.__attoLastResult.actorCompletions');
  assert.equal(closed.length,1,'close completed exactly once');
  assert.deepEqual(closed.map(x=>[x.state,x.admitted,x.delivered,x.dropped]),
    [['CLOSED',1,1,0]]);
  console.log('PASS browser: CLOSED is terminal after prior admission drains and worker acknowledges');
  const separateQueues=senderProgram.replace('  val second = w.send(42);',
    '  val other = spawn Worker();\n  val other_ok = other.send(50);\n  val second = w.send(42);'
  ).replace('first.is_ok() , second.is_ok() , full.is_full()',
    'first.is_ok() , second.is_ok() , other_ok.is_ok() , full.is_full()');
  const twoResult=await run(separateQueues,'main() -> 42');
  assert.match(twoResult,/actor\[1\] receive\(41\) -> 42/);
  assert.match(twoResult,/actor\[2\] receive\(50\) -> 51/);
  console.log('PASS browser: each actor has independent bounded mailbox capacity');
  const independent=await evaluate('window.__attoLastResult.actorCompletions');
  assert.deepEqual(independent.map(x=>[x.handle,x.state,x.admitted,x.delivered,x.dropped]),
    [[1,'FINISHED',2,2,0],[2,'FINISHED',1,1,0]]);
  console.log('PASS browser: both actor workers independently acknowledge completion exactly once');

  // A spawned actor without messages is shut down by the finite run scope.
  await evaluate('window.__attoActorMessages=undefined');
  assert.match(await run(actorExample,'main() -> 0'),/main\(\) -> 0/);
  const idle=await evaluate('window.__attoLastResult.actorCompletions');
  assert.deepEqual(idle.map(x=>[x.state,x.admitted,x.delivered]),
    [['FINISHED',0,0]]);
  console.log('PASS browser: idle actor receives shutdown ACK and one FINISHED completion');

  // The guest receive overflows at runtime after successful mailbox admission.
  // Failure must not be conflated with an Err(Full) or an ordinary shutdown.
  const panicActor=[
    'define actor Worker as',
    '  receive(msg: int) -> int { return msg + 9223372036854775807; }',
    'end',
    'pub fnc main(): int {',
    '  val w = spawn Worker();',
    '  val admitted = w.send(1);',
    '  if admitted.is_ok() then return 0; else return 9; fi',
    '}',
  ].join('\n');
  assert.match(await run(panicActor,'E_ACTOR_PANIC_1'),/E_ACTOR_PANIC_1/);
  const failed=await evaluate('window.__attoLastResult.actorCompletions');
  assert.equal(failed.length,1,'failure completion delivered once');
  assert.equal(failed[0].state,'FAILED');
  assert.equal(failed[0].reason,'E_ACTOR_PANIC_1');
  assert.deepEqual([failed[0].admitted,failed[0].delivered,failed[0].dropped],[1,0,1]);
  console.log('PASS browser: handler panic marks FAILED with one completion and drops in-flight work');

  // Forced host termination is explicitly NOT a child-acknowledged FINISHED.
  await evaluate('window.__attoActorForceTerminate=1');
  assert.match(await run(actorExample,'E_ACTOR_FORCED'),/E_ACTOR_FORCED/);
  const terminated=await evaluate('window.__attoLastResult.actorCompletions');
  assert.equal(terminated.length,1,'forced actor completion delivered once');
  assert.equal(terminated[0].state,'TERMINATED');
  assert.equal(terminated[0].reason,'E_ACTOR_FORCED');
  await evaluate('window.__attoActorForceTerminate=undefined');
  console.log('PASS browser: forcibly terminated worker is marked TERMINATED, never completed');

  // A main() panic after spawning invalidates the host run, not a worker ACK.
  const panicMain=actorExample.replace('  return 0;',
    '  return 9223372036854775807 + 1;');
  assert.match(await run(panicMain,'E_PANIC: integer overflow'),/E_PANIC: integer overflow/);
  const aborted=await evaluate('window.__attoLastResult.actorCompletions');
  assert.equal(aborted.length,1);
  assert.equal(aborted[0].state,'TERMINATED');
  assert.equal(aborted[0].reason,'E_GUEST_MAIN_PANIC');
  console.log('PASS browser: guest panic cleans up child and records termination once');


  // The exact error cause must survive a failure before the ready handshake.
  await evaluate("window.__attoActorFailureStage='init'");
  assert.match(await run(actorExample,'E_ACTOR_TEST_INIT_FAILURE'),/E_ACTOR_TEST_INIT_FAILURE/);
  const initFailure=await evaluate('window.__attoLastResult.actorCompletions');
  assert.equal(initFailure.length,1);
  assert.equal(initFailure[0].state,'FAILED');
  assert.equal(initFailure[0].reason,'E_ACTOR_TEST_INIT_FAILURE');
  await evaluate('window.__attoActorFailureStage=undefined');
  console.log('PASS browser: failed child initialization is FAILED, not a normal completion');

  // Even after a successful empty drain, a child that rejects shutdown
  // cannot be reported as FINISHED without an acknowledgement.
  await evaluate("window.__attoActorFailureStage='shutdown'");
  assert.match(await run(actorExample,'E_ACTOR_TEST_SHUTDOWN_FAILURE'),/E_ACTOR_TEST_SHUTDOWN_FAILURE/);
  const shutdownFailure=await evaluate('window.__attoLastResult.actorCompletions');
  assert.equal(shutdownFailure.length,1);
  assert.equal(shutdownFailure[0].state,'FAILED');
  assert.equal(shutdownFailure[0].reason,'E_ACTOR_TEST_SHUTDOWN_FAILURE');
  await evaluate('window.__attoActorFailureStage=undefined');
  console.log('PASS browser: rejected shutdown cannot yield FINISHED without child ACK');

  assert.ok(requests.some(x=>x.path==='/atto_compiler.wasm'));
  assert.ok(requests.every(x=>x.method==='GET'));
  console.log('PASS browser: no source POST or server-side compilation');
}finally{
  try{ws?.close()}catch{}
  if(chrome?.exitCode===null){chrome.kill('SIGKILL');await sleep(200)}
  await new Promise(resolve=>server.close(resolve));
  await rm(tmp,{recursive:true,force:true,maxRetries:5,retryDelay:50});
}
