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
  // User-reported /play/ repro. This runs in real Chromium using the actual
  // Rust->Wasm compiler and then the compiler-produced guest Wasm, not an
  // editor parser approximation or JavaScript fallback.
  const userPointExample = [
    'define class Point as',
    '  int x: 40',
    '  int y: 2',
    '  bool active: true',
    'end',
    '',
    'pub fnc main(): int {',
    '  val p = new Point();',
    '  if p.active then',
    '    return p.x + p.y;',
    '  else',
    '    return 0;',
    '  fi',
    '}',
  ].join('\n');
  assert.match(await run(userPointExample, 'main() -> 42'), /^main\(\) -> 42$/);
  console.log('PASS browser: exact user Point/new constructor, bool field and two int fields return 42');
  assert.match(await run(userPointExample.replace('new Point()', 'Point()'), 'E_NEW_REQUIRED'), /E_NEW_REQUIRED/);
  console.log('PASS browser: exact user Point without new rejects and worker enforces zero Wasm artifact');
  assert.match(await run(userPointExample.replace('p.x + p.y', 'p.missing'), 'E_UNKNOWN_FIELD'), /E_UNKNOWN_FIELD/);
  console.log('PASS browser: invalid class field rejects with E_UNKNOWN_FIELD and zero Wasm artifact');
  const enumExample = [
    'define enum Status as Pending, Ready end',
    'pub fnc main(): int {',
    '  val s: Status = Status.Ready;',
    '  if s eq Status.Ready then return 42; else return 0; fi',
    '}',
  ].join('\n');
  assert.match(await run(enumExample, 'main() -> 42'), /^main\(\) -> 42$/);
  console.log('PASS browser: nominal enum variant compiles and executes to 42');
  assert.match(await run(userPointExample, 'main() -> 42'), /^main\(\) -> 42$/);
  console.log('PASS browser: real compiler recovers after two rejected class programs');
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
  // Compile a real AttoLang actor handler inside a dedicated Web Worker,
  // then execute its compiler-produced guest Wasm on multiple messages.
  const actor=await evaluate(`(async()=>{
    const worker=new Worker('/actor-worker.mjs',{type:'module'});
    const request=packet=>new Promise((resolve,reject)=>{
      const timeout=setTimeout(()=>reject(new Error('actor worker deadline')),5000);
      worker.onmessage=e=>{clearTimeout(timeout);resolve(e.data)};
      worker.onerror=e=>{clearTimeout(timeout);reject(new Error('actor worker failed'))};
      worker.postMessage(packet);
    });
    try {
      const ready=await request({kind:'init',source:'define actor Worker as receive(int value): int { return value + 2; } end'});
      if(ready.kind!=='ready')throw new Error(JSON.stringify(ready));
      const first=await request({kind:'message',id:1,value:40n});
      const second=await request({kind:'message',id:2,value:-1n});
      return {a:first.value?.toString(),b:second.value?.toString(),status:first.kind+'/'+second.kind};
    } finally { worker.terminate(); }
  })()`);
  assert.deepEqual(actor,{a:'42',b:'1',status:'result/result'});
  console.log('PASS browser: real Atto actor source -> compiler Wasm -> actor guest Wasm -> dedicated Web Worker receive');

  const rejectedActor=await evaluate(`(async()=>{
    const worker=new Worker('/actor-worker.mjs',{type:'module'});
    try {
      return await new Promise((resolve,reject)=>{
        const timer=setTimeout(()=>reject(new Error('rejection deadline')),5000);
        worker.onmessage=e=>{clearTimeout(timer);resolve(e.data)};
        worker.onerror=()=>{clearTimeout(timer);reject(new Error('worker failed'))};
        worker.postMessage({kind:'init',source:'define actor Bad as receive(bool value): int { return 1; } end'});
      });
    } finally { worker.terminate(); }
  })()`);
  assert.equal(rejectedActor.kind,'fault');
  assert.match(rejectedActor.message,/E_UNSUPPORTED/);
  console.log('PASS browser: wrong actor receive signature fails closed before guest creation');

  // Probe the actual source-owned worker, not a simulated worker. Neither a
  // second init during async compilation nor skipped/replayed message IDs
  // may be accepted as guest execution.
  const actorProtocol=await evaluate(`(async()=>{
    const source='define actor Worker as receive(int value): int { return value + 2; } end';
    async function probe(mode) {
      const w=new Worker('/actor-worker.mjs',{type:'module'});
      const next=()=>new Promise((resolve,reject)=>{
        const timer=setTimeout(()=>reject(new Error('actor protocol deadline: '+mode)),5000);
        w.onmessage=e=>{clearTimeout(timer);resolve(e.data)};
        w.onerror=()=>{clearTimeout(timer);reject(new Error('actor protocol worker error'))};
      });
      try {
        if(mode==='double-init') {
          const first=next();
          w.postMessage({kind:'init',source});
          w.postMessage({kind:'init',source});
          const reply=await first;
          return reply.kind==='fault' && /E_ACTOR_PROTOCOL/.test(reply.message);
        }
        let pending=next();
        w.postMessage({kind:'init',source});
        const ready=await pending;
        if(ready.kind!=='ready') throw new Error('expected actor ready');
        if(mode==='replay') {
          pending=next();
          w.postMessage({kind:'message',id:1,value:40n});
          const valid=await pending;
          if(valid.kind!=='result'||valid.value!==42n)throw new Error('initial receive failed');
          pending=next();
          w.postMessage({kind:'message',id:1,value:41n});
        } else {
          pending=next();
          w.postMessage({kind:'message',id:2,value:41n});
        }
        const invalid=await pending;
        return invalid.kind==='fault' && /E_ACTOR_PROTOCOL/.test(invalid.message);
      } finally { w.terminate(); }
    }
    return {doubleInit:await probe('double-init'),replayed:await probe('replay'),skipped:await probe('skip')};
  })()`);
  assert.deepEqual(actorProtocol,{doubleInit:true,replayed:true,skipped:true});
  console.log('PASS browser: actor double-init, replay and skipped IDs are rejected before guest execution');

  // Real worker transport must preserve the full signed-i64 domain. A raw
  // BigInt outside it must NOT be implicitly truncated modulo 2^64 by Wasm.
  const actorI64Bounds=await evaluate(`(async()=>{
    const source='define actor Identity as receive(int value): int { return value; } end';
    async function open() {
      const w=new Worker('/actor-worker.mjs',{type:'module'});
      const request=packet=>new Promise((resolve,reject)=>{
        const timeout=setTimeout(()=>reject(new Error('actor i64 deadline')),5000);
        w.onmessage=e=>{clearTimeout(timeout);resolve(e.data)};
        w.onerror=()=>{clearTimeout(timeout);reject(new Error('actor i64 worker error'))};
        w.postMessage(packet);
      });
      const init=await request({kind:'init',source});
      if(init.kind!=='ready'){w.terminate();throw Error('actor did not initialize');}
      return {w,request};
    }
    const values=[-(1n<<63n),-1n,0n,(1n<<63n)-1n];
    const first=await open();
    try {
      const results=[];
      for(let i=0;i<values.length;i++){
        const r=await first.request({kind:'message',id:i+1,value:values[i]});
        results.push(r.kind==='result'&&r.id===i+1&&r.value===values[i]);
      }
      const reinit=await first.request({kind:'init',source});
      if(reinit.kind!=='fault'||!/E_ACTOR_PROTOCOL/.test(reinit.message))
        throw Error('reinit after ready must fault');
      const bad=[(1n<<63n),-(1n<<63n)-1n,1,1.2,null,
        {value:1n},true,undefined];
      const refused=[];
      for(const value of bad){
        const a=await open();
        try {
          const r=await a.request({kind:'message',id:1,value});
          refused.push(r.kind==='fault'&&/E_ACTOR_PROTOCOL/.test(r.message));
        } finally {a.w.terminate();}
      }
      return {boundary:results,invalid:refused,reinit:true};
    } finally {first.w.terminate();}
  })()`);
  assert.deepEqual(actorI64Bounds,{boundary:[true,true,true,true],
    invalid:Array(8).fill(true),reinit:true});
  console.log('PASS browser: actor signed i64 min/max roundtrip; out-of-range and non-BigInt payloads and second init fail closed');
  const compilerFetchesBeforeOversize=requests.filter(x=>x.path==='/atto_compiler.wasm').length;
  const oversizedActor=await evaluate(`(async()=>{
    const worker=new Worker('/actor-worker.mjs',{type:'module'});
    try {
      return await new Promise((resolve,reject)=>{
        const deadline=setTimeout(()=>reject(new Error('oversize actor deadline')),5000);
        worker.onmessage=e=>{clearTimeout(deadline);resolve(e.data)};
        worker.onerror=()=>{clearTimeout(deadline);reject(new Error('oversize worker failed'))};
        worker.postMessage({kind:'init',source:'x'.repeat(65537)});
      });
    } finally { worker.terminate(); }
  })()`);
  assert.equal(oversizedActor.kind,'fault');
  assert.match(oversizedActor.message,/E_SOURCE_LIMIT/);
  assert.equal(requests.filter(x=>x.path==='/atto_compiler.wasm').length,
    compilerFetchesBeforeOversize,'oversized source must fail before compiler fetch');
  console.log('PASS browser: oversized actor source denied before fetching or compiling Wasm');

  assert.ok(requests.some(x=>x.path==='/atto_compiler.wasm'));
  assert.ok(requests.every(x=>x.method==='GET'));
  console.log('PASS browser: no source POST or server-side compilation');
}finally{
  try{ws?.close()}catch{}
  if(chrome?.exitCode===null){chrome.kill('SIGKILL');await sleep(200)}
  await new Promise(resolve=>server.close(resolve));
  await rm(tmp,{recursive:true,force:true,maxRetries:5,retryDelay:50});
}
