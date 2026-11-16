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
  assert.ok(requests.some(x=>x.path==='/atto_compiler.wasm'));
  assert.ok(requests.every(x=>x.method==='GET'));
  console.log('PASS browser: no source POST or server-side compilation');
}finally{
  try{ws?.close()}catch{}
  if(chrome?.exitCode===null){chrome.kill('SIGKILL');await sleep(200)}
  await new Promise(resolve=>server.close(resolve));
  await rm(tmp,{recursive:true,force:true,maxRetries:5,retryDelay:50});
}
