// Real Chromium tenant worker + real compiler-emitted guest actor pipeline.
// This tests only the explicitly scoped pipeline-v1 extension, not actor_resume.
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { spawn } from 'node:child_process';
import { readFile, mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const site = process.env.ATTO_PLAY_SITE_ROOT;
if (!site) throw new Error('ATTO_PLAY_SITE_ROOT must be exact reviewed site checkout');
const compilerModule = await import(pathToFileURL(join(resolve(site), 'play/src/compiler.js')).href);
const compiler = await compilerModule.createCompiler(await readFile(
  process.argv[2] ?? 'target/wasm32-unknown-unknown/release/atto_compiler.wasm'));
const source = `
define actor Parse as
  receive(ActorMail<int> mail): void { emit(mail.value + 1); }
end
define actor Validate as
  receive(ActorMail<int> mail): void {
    if mail.value > 0 then emit(mail.value); fi
  }
end
define actor Sink as
  receive(ActorMail<int> mail): void { stdio.println(mail.value); }
end
pub fnc main(): int {
  val pipeline = spawn Parse() pipe spawn Validate() pipe spawn Sink();
  pipeline.send(41);
  return 0;
}`;
const generated = compiler.compile(source);
assert.equal(generated.ok, true, JSON.stringify(generated.diagnostics));
const guest = new Uint8Array(generated.wasm);
assert.equal(WebAssembly.validate(guest), true);
const mod = new WebAssembly.Module(guest);
assert.deepEqual([...new Uint8Array(WebAssembly.Module.customSections(mod,'atto.pipeline.v1')[0])],[1]);

const leb = n => {const b=[]; do {let x=n&127;n>>>=7;if(n)x|=128;b.push(x);}while(n);return b;};
const name = s => {const b=[...new TextEncoder().encode(s)];return [...leb(b.length),...b];};
const section = (id,data) => [id,...leb(data.length),...data];
const surplusStages = Array.from({length:5},(_,i)=>
  'val extra' + i + ' = spawn Parse() pipe spawn Validate() pipe spawn Sink();'
).join('\\n  ');
const quotaSource = source.replace('pipeline.send(41);',
  surplusStages + '\\n  pipeline.send(41);');
const quotaCompiled = compiler.compile(quotaSource);
assert.equal(quotaCompiled.ok, true, JSON.stringify(quotaCompiled.diagnostics));
const overQuotaGuest = new Uint8Array(quotaCompiled.wasm);
assert.equal(WebAssembly.validate(overQuotaGuest), true);
const v3Forged = new Uint8Array([...guest,...section(0,[...name('atto.actor.abi'),3,0])]);
assert.equal(WebAssembly.validate(v3Forged), true, 'ABI refusal needs valid core Wasm');

const staticJs = new Set(['runner-worker.js','actor-host.js','actor-abi-version.js',
  'wasm-memory-admission.js','guest.js']);
const requests = [];
const server = createServer(async(req,res)=>{
  requests.push(req.method+' '+req.url);
  if(req.method!=='GET'){res.writeHead(405);res.end();return;}
  try {
    let body,kind;
    const path = new URL(req.url,'http://localhost').pathname;
    if(path==='/index.html'){body='<!doctype html><title>Atto actor worker contract</title>';kind='text/html';}
    else if(path==='/guest.wasm'){body=guest;kind='application/wasm';}
    else if(path==='/guest-v3.wasm'){body=v3Forged;kind='application/wasm';}
    else if(path==='/guest-over-quota.wasm'){body=overQuotaGuest;kind='application/wasm';}
    else if(path.startsWith('/play/src/')&&staticJs.has(path.slice('/play/src/'.length))){
      body=await readFile(join(resolve(site),'play/src',path.slice('/play/src/'.length)));
      kind='text/javascript';
    } else {res.writeHead(404);res.end();return;}
    res.writeHead(200,{'content-type':kind,'x-content-type-options':'nosniff','cache-control':'no-store'});
    res.end(body);
  }catch(error){res.writeHead(500);res.end(String(error.message));}
});
const sleep=ms=>new Promise(r=>setTimeout(r,ms));
async function until(fn,label){let detail;for(let i=0;i<400;i++){try{const v=await fn();if(v)return v;detail=v;}catch(e){detail=e.message;}await sleep(100);}throw new Error(label+' timeout: '+detail);}
const tmp=await mkdtemp(join(tmpdir(),'atto-actor-browser-'));
let chrome,ws;
try {
  await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
  const browser=process.env.CHROME_BIN||'/usr/bin/google-chrome';
  chrome=spawn(browser,['--headless=new','--no-sandbox','--disable-gpu','--disable-dev-shm-usage',
    '--no-first-run','--no-proxy-server','--disable-background-networking',
    '--remote-debugging-port=0','--user-data-dir='+tmp,'about:blank'],{stdio:['ignore','ignore','pipe']});
  let stderr='';chrome.stderr.on('data',x=>stderr=(stderr+x.toString()).slice(-1600));
  const port=await until(async()=>{
    if(chrome.exitCode!==null)throw new Error('Chrome exited: '+stderr);
    try{return Number((await readFile(join(tmp,'DevToolsActivePort'),'utf8')).split('\n')[0])||null;}catch{return null;}
  },'Chromium debugging');
  const targets=await (await fetch('http://127.0.0.1:'+port+'/json/list')).json();
  const page=targets.find(x=>x.type==='page'&&x.webSocketDebuggerUrl);
  assert.ok(page,'Chromium page target must exist');
  ws=new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((res,rej)=>{ws.addEventListener('open',res,{once:true});ws.addEventListener('error',rej,{once:true});});
  const pending=new Map();let seq=0;
  ws.addEventListener('message',e=>{
    const r=JSON.parse(e.data);if(!pending.has(r.id))return;
    const call=pending.get(r.id);pending.delete(r.id);
    if(r.error)call.reject(new Error(r.error.message));else call.resolve(r.result);
  });
  const command=(method,params={})=>new Promise((resolve,reject)=>{
    const id=++seq;pending.set(id,{resolve,reject});ws.send(JSON.stringify({id,method,params}));
  });
  const evalJs=async code=>{
    const x=await command('Runtime.evaluate',{expression:code,returnByValue:true,awaitPromise:true});
    if(x.exceptionDetails)throw new Error(x.exceptionDetails.exception?.description||x.exceptionDetails.text);
    return x.result?.value;
  };
  await command('Page.enable');await command('Runtime.enable');
  await command('Page.navigate',{url:'http://127.0.0.1:'+server.address().port+'/index.html'});
  await until(()=>evalJs("document.readyState === 'complete'"),'browser ready');
  const workerCase=async path=>evalJs(`new Promise(async (resolve,reject)=>{
    const w=new Worker('/play/src/runner-worker.js',{type:'module'});
    w.onmessage=e=>{w.terminate();resolve(e.data)};
    w.onerror=e=>{w.terminate();reject(new Error('Worker: '+e.message))};
    try {const response=await fetch(${JSON.stringify(path)});
      if(!response.ok)throw new Error('Wasm fetch '+response.status);
      w.postMessage({wasm:await response.arrayBuffer()});
    }catch(e){w.terminate();reject(e)}
  })`);
  const good=await workerCase('/guest.wasm');
  assert.equal(good.ok,true,JSON.stringify(good));
  assert.equal(good.value,'0');
  assert.equal(good.actors,3);
  assert.equal(good.stdout,'42\n');
  console.log('PASS Chromium: Rust-compiled pipeline sends 42 through three isolated actor Wasm instances');

  const blocked=await workerCase('/guest-v3.wasm');
  assert.equal(blocked.ok,false);
  assert.match(blocked.message,/^E_GUEST_ABI_UNSUPPORTED: actor_resume v3/);
  assert.equal(blocked.actors,0);
  assert.equal(blocked.stdout,'');
  console.log('PASS Chromium: forged full actor-resume ABI v3 rejected before actor allocation');
  const full=await workerCase('/guest-over-quota.wasm');
  assert.equal(full.ok,false);
  assert.match(full.message,/^E_ACTOR_LIMIT: at most 16 actors per run/);
  assert.equal(full.stdout,'','failed 17th spawn must not run queued actor mail');
  assert.equal(full.actors,16,'host must stop admitting before allocating an actor beyond quota');
  console.log('PASS Chromium: six pipelines fail actor quota at 16 without executing guest mail');

  assert.ok(requests.every(s=>s.startsWith('GET ')),'browser worker never posts source to server');
}finally{
  try{ws?.close()}catch{}
  if(chrome?.exitCode===null){chrome.kill('SIGKILL');await sleep(200);}
  await new Promise(resolve=>server.close(resolve));
  await rm(tmp,{recursive:true,force:true,maxRetries:5,retryDelay:50});
}
