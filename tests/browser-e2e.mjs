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
  ['/atto_compiler.wasm',[new URL('../target/wasm32-unknown-unknown/release/atto_compiler.wasm',import.meta.url),'application/wasm']],
  ['/m9-supervisor-host.mjs',[new URL('../runtime/m9-supervisor-host.mjs',import.meta.url),'text/javascript; charset=utf-8']],
  ['/m9-actor-worker.mjs',[new URL('../runtime/m9-actor-worker.mjs',import.meta.url),'text/javascript; charset=utf-8']],
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
  // M9: actual Chromium Worker executes the Rust compiler-emitted .atto actor.
  const m9Source=await readFile(new URL('./fixtures/m8/cooperative_sum.atto',import.meta.url),'utf8');
  const m9Lines=[
    '(async()=>{',
    'const {M9SupervisorHost}=await import("/m9-supervisor-host.mjs");',
    'const wasm=await (await fetch("/atto_compiler.wasm")).arrayBuffer();',
    'const c=new WebAssembly.Instance(new WebAssembly.Module(wasm)).exports;',
    'const source='+JSON.stringify(m9Source)+';',
    'function compile(s){const b=new TextEncoder().encode(s);new Uint8Array(c.memory.buffer,c.input_ptr(),b.length).set(b);',
    'if(c.compile(b.length)!==0)throw new Error("M9 source compile failed");',
    'return new Uint8Array(c.memory.buffer,c.result_ptr(),c.result_length()).slice();}',
    'const valid=compile(source);',
    'const invalid=compile(source.replace("let total: int = 0;","let total: int = 9223372036854775807;").replace("let i: int = 0;","let i: int = 1;"));',
    'const notices=[];const host=new M9SupervisorHost({maxActors:3,mailboxCapacity:3,reductionBudget:4,onControl:x=>notices.push(x)});',
    'try {',
    'const a=host.spawn(new Worker("/m9-actor-worker.mjs",{type:"module"}),valid,11);',
    'const b=host.spawn(new Worker("/m9-actor-worker.mjs",{type:"module"}),invalid,11);',
    'for(const n of [1000,12,3])if(host.try_send(a,n)!=="ACCEPTED")throw new Error("healthy admission");',
    'for(const n of [3,3,3])if(host.try_send(b,n)!=="ACCEPTED")throw new Error("fault admission");',
    'if(host.try_send(b,3)!=="FULL")throw new Error("bounded DATA gate");',
    'host.request_stop(b);host.request_stop(a);',
    'if(!(await host.await_terminal(a,15000))||!(await host.await_terminal(b,15000)))throw new Error("worker terminal deadline");',
    'const sa=host.check_conservation(a),sb=host.check_conservation(b);',
    'return {accepted:sa.accepted,completed:sa.completed,sum:String(sa.resultSum),yielded:sa.yielded,',
    'failed:sb.failed,abandoned:sb.abandoned,controls:notices.filter(n=>n.actor_id===b.actor_id).map(n=>n.kind),',
    'fault:notices.find(n=>n.actor_id===b.actor_id&&n.kind==="PANIC")?.fault_code};',
    '} finally{await host.close()}',
    '})()'
  ];
  const m9Result=await evaluate(m9Lines.join('\n'));
  assert.deepEqual(m9Result, {accepted:3,completed:3,sum:'499569',yielded:251,
    failed:1,abandoned:2,controls:['STOP','PANIC'],fault:1});
  assert.ok(requests.some(x=>x.path==='/m9-actor-worker.mjs'));
  console.log('M9_REAL_CHROMIUM_COMPILER_WORKER_SUPERVISOR=PASS');
  assert.ok(requests.some(x=>x.path==='/atto_compiler.wasm'));
  assert.ok(requests.every(x=>x.method==='GET'));
  console.log('PASS browser: no source POST or server-side compilation');
}finally{
  try{ws?.close()}catch{}
  if(chrome?.exitCode===null){chrome.kill('SIGKILL');await sleep(200)}
  await new Promise(resolve=>server.close(resolve));
  await rm(tmp,{recursive:true,force:true,maxRetries:5,retryDelay:50});
}
