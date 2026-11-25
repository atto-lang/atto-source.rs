// Reconciled Rust compiler Wasm + guest Wasm async/JSPI acceptance in actual Chromium.\n// This is a separate harness: older browser fixtures are left untouched.
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {spawn} from 'node:child_process';
import {mkdtemp,readFile,rm} from 'node:fs/promises';
import {join} from 'node:path';
import {tmpdir} from 'node:os';
const root=new URL('.',import.meta.url), tmp=await mkdtemp(join(tmpdir(),'atto-rust-browser-'));
const requests=[];
const files=new Map([
  ['/index.html',[new URL('./browser-async/index.html',import.meta.url),'text/html; charset=utf-8']],
  ['/worker.mjs',[new URL('./browser-async/worker.mjs',import.meta.url),'text/javascript; charset=utf-8']],
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
  const values='fnc label(int n): string {\n  match n over\n    on 0 -> { return "zero"; }\n    on _ -> { return "many"; }\n  end\n}\npub fnc main(): int {\n  val f: float = 7.0 / 2;\n  stdio.println(f);\n  stdio.println(1e7);\n  stdio.println(label(0) + "/" + label(3));\n  stdio.println("  pad ".trim() eq "pad");\n  return 1;\n}';
  assert.match(await run(values,'main() -> 1'),/^3\.5\n1\.0E7\nzero\/many\ntrue\nmain\(\) -> 1$/);
  console.log('PASS browser: float (Java Double.toString), string values and match');
  const readyAwait='pub suspends fnc main(): int { await 42; return await 42; }';
  assert.match(await run(readyAwait,'main() -> 42'),/^main\(\) -> 42$/);
  assert.equal(await evaluate('window.__attoLastCooperativeYields'),2,'two immediate awaits must both yield the browser worker');
  console.log('PASS await: await 42 yields a real JSPI Worker turn twice, while still returning 42');
  const readyMethod='pub define class Worker as pub suspends mtd answer(): int { await 21; return await 42; } end pub suspends fnc main(): int { val worker=new Worker(); return await worker.answer(); }';
  assert.match(await run(readyMethod,'main() -> 42'),/^main\(\) -> 42$/);
  assert.equal(await evaluate('window.__attoLastCooperativeYields'),3,'class mtd and caller awaits must cooperatively yield');
  console.log('PASS await: suspends mtd and caller both enforce cooperative yielding');
  const badClass='pub define class Worker as pub suspends mtd answer(): int { return await 42; } end pub fnc main(): int { val worker=new Worker(); return worker.answer(); }';
  assert.match(await run(badClass,'E_EFFECT'),/E_EFFECT/);
  console.log('PASS await: non-suspending caller of suspends mtd rejected without guest execution');
  const timer='suspends fnc delayedAnswer(): int { await rt.sleep(25); return 42; } pub suspends fnc main(): int { val result=await delayedAnswer(); stdio.println(result); return result; }';
  assert.match(await run(timer,'main() -> 42'),/^42\nmain\(\) -> 42$/);
  console.log('PASS JSPI: actual Wasm timer suspended, then resumed after host timer');
  const tasks='pub suspends fnc main(): int { val a=task {return 20;}; val b=task {return 22;}; val c=await a + await b; stdio.println(c); return c; }';
  assert.match(await run(tasks,'main() -> 42'),/^42\nmain\(\) -> 42$/);
  console.log('PASS JSPI: two compiler-generated Wasm task functions produce and await typed Future<int>');
  const barrier='pub suspends fnc main(): int { val a=task { await rt.barrier(2); return 20; }; val b=task { await rt.barrier(2); return 22; }; return await a + await b; }';
  assert.match(await run(barrier,'main() -> 42'),/^main\(\) -> 42$/);
  console.log('PASS JSPI: two Wasm task continuations overlap at strict two-party barrier (not eager/sequential)');
  // New typed-arm JSPI admission: selected arm launches/awaits exactly one task.
  const typedArm='pub suspends fnc main(): int { return match<int> 1 over on 1 : int -> await task { return 42; }, on _ : int -> await task { return 0; }, end; }';
  assert.match(await run(typedArm,'main() -> 42'),/^main\(\) -> 42$/);
  assert.equal(await evaluate('window.__attoLastCooperativeYields'),1,'only selected typed arm awaits once');
  console.log('PASS integration: typed match<int> arm executes awaited task with one scheduler handoff');
  const badTypedArm='pub suspends fnc main(): int { return match<int> 1 over on 1 : bool -> true, on _ : int -> 0, end; }';
  assert.match(await run(badTypedArm,'E_TYPE'),/E_TYPE/);
  console.log('PASS integration: conflicting typed arm fails closed without a guest');
  // Switch/case uses the same checked match lowering and JSPI continuation path.
  const typedCase='pub suspends fnc main(): int { return switch<int> 1 over case 1 : int -> await task { return 42; }, default : int -> await task { return 0; }, end; }';
  assert.match(await run(typedCase,'main() -> 42'),/^main\(\) -> 42$/);
  assert.equal(await evaluate('window.__attoLastCooperativeYields'),1,'only selected switch case awaits once');
  console.log('PASS integration: typed switch<int> case executes awaited task with one scheduler handoff');
  const discardedErr='fnc checked(): Result<int,string> { return Ok(41); } pub suspends fnc main(): int { return match<int> checked() over on Err() : int -> 0, on Ok(value) : int -> await task { return value + 1; }, end; }';
  assert.match(await run(discardedErr,'main() -> 42'),/^main\(\) -> 42$/);
  assert.equal(await evaluate('window.__attoLastCooperativeYields'),1);
  console.log('PASS browser: Err() discards payload while Ok(value) binds and task captures it');
  const discardedOk='fnc checked(): Result<int,string> { return Ok(41); } pub fnc main(): int { return switch<int> checked() over case Err(_) : int -> -1, case Ok() : int -> 42, end; }';
  assert.match(await run(discardedOk,'main() -> 42'),/^main\(\) -> 42$/);
  console.log('PASS browser: Ok() and Err(_) discard tagged payloads');
  const badDiscardReference='fnc checked(): Result<int,string> { return Ok(41); } pub fnc main(): int { return match<int> checked() over on Err() : int -> 0, on Ok() : int -> value + 1, end; }';
  assert.match(await run(badDiscardReference,'E_UNBOUND'),/E_UNBOUND/);
  console.log('PASS browser: Ok() does not create an implicit value local');
  const guardedDiscard='fnc checked(): Result<int,string> { return Ok(41); } pub fnc main(): int { return match<int> checked() over on Ok() when false : int -> 7, on Ok(n) when n > 40 : int -> n + 1, on Ok(_) : int -> 0, on Err() : int -> -1, end; }';
  assert.match(await run(guardedDiscard,'main() -> 42'),/^main\(\) -> 42$/);
  console.log('PASS browser: guarded Ok() falls through and named Ok(n) guard observes the tagged payload');
  const guardedErrDiscard='fnc checked(): Result<int,string> { return Ok(41); } pub fnc main(): int { return switch<int> checked() over case Err(message) when message == "wrong" : int -> 1, case Err() : int -> 0, case Ok(_) : int -> 42, end; }';
  assert.match(await run(guardedErrDiscard,'main() -> 42'),/^main\(\) -> 42$/);
  console.log('PASS browser: unselected guarded Err does not read the active Ok payload');

  // A named guard must observe its initialized Ok payload; the selected
  // awaited task captures only the copied int, and no unselected task runs.
  const guardedCapturedTask='fnc checked(): Result<int,string> { return Ok(41); } pub suspends fnc main(): int { return match<int> checked() over on Err() : int -> 0, on Ok(value) when value > 5 : int -> await task { return value + 1; }, on Ok(_) : int -> 10, end; }';
  assert.match(await run(guardedCapturedTask,'main() -> 42'),/^main\(\) -> 42$/);
  assert.equal(await evaluate('window.__attoLastCooperativeYields'),1,'guarded named capture must suspend once');
  console.log('PASS browser: guarded Ok(value) copies int into selected awaited task');
  const guardedDiscardTask='fnc checked(): Result<int,string> { return Ok(2); } pub suspends fnc main(): int { return switch<int> checked() over case Err(_) : int -> 0, case Ok(value) when value > 5 : int -> await task { return value + 1; }, case Ok() : int -> await task { return 42; }, end; }';
  assert.match(await run(guardedDiscardTask,'main() -> 42'),/^main\(\) -> 42$/);
  assert.equal(await evaluate('window.__attoLastCooperativeYields'),1,'failed guard must not schedule an unselected task');
  console.log('PASS browser: guarded fallthrough to Ok() schedules only selected task');
  const invalidDiscardTask='fnc checked(): Result<int,string> { return Ok(41); } pub suspends fnc main(): int { return match<int> checked() over on Err() : int -> 0, on Ok() : int -> await task { return value + 1; }, end; }';
  assert.match(await run(invalidDiscardTask,'E_UNBOUND'),/E_UNBOUND/);
  console.log('PASS browser: Ok() does not create a capture for an awaited task');

  const capturedMatch='fnc checked(): Option<int> { return Some(41); } pub suspends fnc main(): int { return match<int> checked() over on Some(n) : int -> await task { return n + 1; }, on None : int -> 0, end; }';
  assert.match(await run(capturedMatch,'main() -> 42'),/^main\(\) -> 42$/);
  assert.equal(await evaluate('window.__attoLastCooperativeYields'),1,'captured match must yield once');
  console.log('PASS JSPI v3: typed match arm copies i64 payload into scheduled task');
  const capturedSwitch='fnc checked(): Result<int,string> { return Ok(41); } pub suspends fnc main(): int { return switch<int> checked() over case Err(message) : int -> 0, case Ok(value) : int -> await task { return value + 1; }, end; }';
  assert.match(await run(capturedSwitch,'main() -> 42'),/^main\(\) -> 42$/);
  assert.equal(await evaluate('window.__attoLastCooperativeYields'),1,'captured switch must yield once');
  console.log('PASS JSPI v3: typed switch arm copies i64 payload into scheduled task');
  const badCaptured='fnc checked(): Result<int,string> { return Err("no"); } pub suspends fnc main(): int { return match<int> checked() over on Err(error) : int -> await task { return error; }, on Ok(value) : int -> value, end; }';
  assert.match(await run(badCaptured,'E_UNSUPPORTED'),/E_UNSUPPORTED/);
  console.log('PASS JSPI v3: string task capture is rejected without guest instantiation');
  const conflictingCase='pub suspends fnc main(): int { return switch<int> 1 over case 1 : bool -> true, default : int -> 0, end; }';
  assert.match(await run(conflictingCase,'E_TYPE'),/E_TYPE/);
  console.log('PASS integration: typed switch case mismatch fails closed without a guest');
  // The latest switch integration must not regress scalar arm-local task captures.
  const legacySwitch='pub fnc main(): int { switch 1 case 1 -> { return 42; } default -> { return 0; } end }';
  assert.match(await run(legacySwitch,'main() -> 42'),/^main\(\) -> 42$/);
  console.log('PASS integration: Java-compatible untyped legacy switch remains supported');
  const guardedCaptureCase='fnc choice(): Result<int,string> { return Ok(41); } pub suspends fnc main(): int { return switch<int> choice() over case Err(error) : int -> -3, case Ok(value) : int -> await task { return value + 1; }, end; }';
  assert.match(await run(guardedCaptureCase,'main() -> 42'),/^main\(\) -> 42$/);
  assert.equal(await evaluate('window.__attoLastCooperativeYields'),1,'selected ADT case captures one int and awaits exactly once');
  console.log('PASS integration: updated typed switch, scalar captured Result payload and JSPI handoff');
  const unsupportedTaskCall='pub suspends fnc main(): int { return await task(); }';
  assert.match(await run(unsupportedTaskCall,'E_UNSUPPORTED'),/E_UNSUPPORTED/);
  console.log('PASS integration: task() remains unsupported, not a task block');
  const combined='pub suspends fnc main(): int { val a=task { return 42; }; stdio.out << "direct"; return await a; }';
  assert.match(await run(combined,'main() -> 42'),/^directmain\(\) -> 42$/);
  assert.equal(await evaluate('window.__attoLastCooperativeYields'),1,'combined stdout/Future guest must yield exactly once');
  console.log('PASS integration: direct stdio.out << and awaited Future<int> execute together inside Chromium Worker');
  for(const [source,code] of [
    ['pub suspends fnc main(): int { nb stdio.out << "queued"; return await 42; }','E_UNSUPPORTED'],
    ['pub suspends fnc main(): int { do writech stdio.out, "queued"; return await 42; }','E_UNSUPPORTED'],
    ['pub suspends fnc main(): int { do readch stdio.in; return await 42; }','E_IO_DISCARD'],
  ]){
    assert.match(await run(source,code),new RegExp(code));
  }
  console.log('PASS integration: unsupported nb/do guests and discarded reads fail closed');
  assert.ok(requests.some(x=>x.path==='/atto_compiler.wasm'));
  assert.ok(requests.every(x=>x.method==='GET'));
  console.log('PASS browser: no source POST or server-side compilation');
}finally{
  try{ws?.close()}catch{}
  if(chrome?.exitCode===null){chrome.kill('SIGKILL');await sleep(200)}
  await new Promise(resolve=>server.close(resolve));
  await rm(tmp,{recursive:true,force:true,maxRetries:5,retryDelay:50});
}
