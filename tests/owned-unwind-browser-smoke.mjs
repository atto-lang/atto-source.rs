// Compile actual .atto with native Rust attoc and serve only explicit assets.
import {createServer} from 'node:http';
import {spawn,spawnSync} from 'node:child_process';
import {readFile,mkdtemp,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join,resolve} from 'node:path';
const js=process.env.ATTO_JS_SOURCE;
if(!js)throw Error('ATTO_JS_SOURCE required');
const root=resolve('.');
const profile=await mkdtemp(join(tmpdir(),'atto-rust-unwind-chrome-'));
const delay=ms=>new Promise(r=>setTimeout(r,ms));
const wasmPath=join(profile,'recovery.wasm');
const built=spawnSync('target/debug/attoc',['--owned-unwind-v1','tests/fixtures/owned-pending-recover.atto','-o',wasmPath],{encoding:'utf8'});
if(built.status!==0){await rm(profile,{recursive:true,force:true});throw Error(built.stderr);}
const assets=new Map([
 ['/tests/owned-unwind-browser.html',[join(root,'tests/owned-unwind-browser.html'),'text/html; charset=utf-8']],
 ['/tests/fixtures/owned-unwind-browser-worker.mjs',[join(root,'tests/fixtures/owned-unwind-browser-worker.mjs'),'text/javascript; charset=utf-8']],
 ['/runtime/browser/owned-unwind-host-v1.mjs',[join(root,'runtime/browser/owned-unwind-host-v1.mjs'),'text/javascript; charset=utf-8']],
 ['/guest.wasm',[wasmPath,'application/wasm']],
 ...["worker-control-loop.mjs","select-continuation-fence.mjs","scheduled-unwind-runtime.mjs","unwind-context.mjs","selectable-i64-future.mjs"].map(p=>['/js/src/'+p,[resolve(js,'src',p),'text/javascript; charset=utf-8']])
]);
const contents=new Map(await Promise.all([...assets].map(async([url,[path,type]])=>[url,[await readFile(path),type]])));
const server=createServer((req,res)=>{
 const entry=contents.get(req.url);
 if(req.method!=='GET'||!entry){res.writeHead(404);res.end();return;}
 res.writeHead(200,{'Content-Type':entry[1],'Cache-Control':'no-store','X-Content-Type-Options':'nosniff','Content-Security-Policy':"default-src 'none'; script-src 'self' 'unsafe-inline' 'wasm-unsafe-eval'; connect-src 'self'; worker-src 'self'; base-uri 'none'; object-src 'none'"});
 res.end(entry[0]);
});
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
const chrome=spawn(process.env.CHROME_BIN||'google-chrome',[
  '--headless=new','--no-sandbox','--disable-dev-shm-usage','--no-first-run',
  '--disable-background-networking','--remote-debugging-port=0',
  '--user-data-dir='+profile,'about:blank'
],{stdio:['ignore','ignore','pipe']});
let chromeError=null,exited=false,ws;
let chromeStderr='';
chrome.stderr.on('data',chunk=>{chromeStderr=(chromeStderr+chunk).slice(-16000);});
chrome.on('exit',()=>{exited=true;});
chrome.on('error',err=>{chromeError=err;exited=true;});
try{
  let port;
  for(let i=0;i<250;i++){
    if(exited)throw Error('Chromium startup failed: '+chromeError+' '+chromeStderr);
    try{port=Number((await readFile(join(profile,'DevToolsActivePort'),'utf8')).split('\n')[0]);if(port)break;}catch{}
    await delay(100);
  }
  if(!port)throw Error('Chromium DevTools startup timeout');
  const pages=await(await fetch('http://127.0.0.1:'+port+'/json/list')).json();
  const page=pages.find(p=>p.type==='page');
  if(!page)throw Error('no Chromium page target');
  ws=new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((ok,fail)=>{ws.addEventListener('open',ok,{once:true});ws.addEventListener('error',fail,{once:true});});
  const inflight=new Map();let serial=0;
  ws.addEventListener('message',event=>{
    const message=JSON.parse(event.data),entry=inflight.get(message.id);
    if(entry){inflight.delete(message.id);message.error?entry.reject(Error(JSON.stringify(message.error))):entry.resolve(message.result);}
  });
  const rpc=(method,params={})=>new Promise((resolve,reject)=>{
    const id=++serial;inflight.set(id,{resolve,reject});ws.send(JSON.stringify({id,method,params}));
  });
  await rpc('Page.enable');await rpc('Runtime.enable');
  await rpc('Page.navigate',{url:'http://127.0.0.1:'+server.address().port+'/tests/owned-unwind-browser.html'});
  let status='';
  for(let i=0;i<180;i++){
    await delay(100);
    const answer=await rpc('Runtime.evaluate',{expression:"document.querySelector('#result')?.textContent",returnByValue:true});
    status=answer?.result?.value??'';
    if(status.startsWith('PASS_')||status.startsWith('FAIL_'))break;
  }
  console.log('Rust AOT browser Worker: '+(status||'no result'));
  if(status!=='PASS_RUST_OWNED_UNWIND_WORKER')throw Error('real browser Worker control-loop smoke failed');
}finally{
  ws?.close();chrome.kill('SIGTERM');server.closeAllConnections();
  await new Promise(resolve=>server.close(resolve));
  if(!exited)await Promise.race([new Promise(r=>chrome.once('exit',r)),delay(1500)]);
  await rm(profile,{recursive:true,force:true,maxRetries:8,retryDelay:250}).catch(()=>{});
}
