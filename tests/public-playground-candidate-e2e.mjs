// Exact-snapshot public playground IDE + PRIVATE compiler candidate in Chromium.
// This test never uploads, vendors, releases, or commits the candidate binary.
// The source repo CI builds candidate Wasm locally; the playground is public,
// source-only checkout at an immutable reviewed Git SHA.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { execFileSync, spawn } from 'node:child_process';
import { createServer } from 'node:http';
import { cp, mkdir, mkdtemp, readFile, readdir, rm, stat, writeFile } from 'node:fs/promises';
import { join, resolve, sep, extname } from 'node:path';
import { tmpdir } from 'node:os';

const PUBLIC_SITE_COMMIT = 'ced47338040427e824728fc96ffeb5906982efe6';
const root = resolve(new URL('..', import.meta.url).pathname);
const publicCheckout = join(root, 'public-site-checkout');
const compilerPath = join(root, 'target/wasm32-unknown-unknown/release/atto_compiler.wasm');
const assets = [
  'index.html', 'styles.css',
  'src/app.js', 'src/compiler.js', 'src/compiler-worker.js', 'src/runner-worker.js',
  'src/guest.js', 'src/worker-task.js', 'src/editor.js', 'src/ui.js',
  'src/actor-lifecycle.js', 'src/actor-guest-worker.js',
  'src/wasm-admission.js',
  'src/program-picker.js', 'src/saved-programs.js', 'src/syntax-highlight.js',
  'src/formatter-worker.js', 'src/formatter-runtime.js', 'src/formatter-pin.js',
  'src/compiler-pin.js', 'public/atto_compiler.wasm', 'public/atto_compiler.receipt.json',
  'public/oreslang_format.wasm', 'public/oreslang_format.receipt.json',
  'docs/COMPILER_CONTRACT.md', 'docs/BROWSER_ACTOR_LIFECYCLE.md',
  'examples/conditions.atto', 'examples/error.atto', 'examples/fib.atto',
  'examples/hello.atto', 'examples/overflow.atto', 'examples/primes.atto'
];
const mime = {'.html':'text/html','.css':'text/css','.js':'text/javascript','.wasm':'application/wasm','.atto':'text/plain','.md':'text/plain','.json':'application/json'};
const git = (...args) => execFileSync('git', args, {encoding:'utf8'}).trim();
assert.equal(git('-C', publicCheckout, 'rev-parse', 'HEAD'), PUBLIC_SITE_COMMIT,
  'unexpected public frontend source: exact snapshot required');
const sourceHead = git('rev-parse', 'HEAD');
const sourceTree = git('rev-parse', 'HEAD^{tree}');
assert.match(sourceHead, /^[0-9a-f]{40}$/);
const bytes = await readFile(compilerPath);
assert.ok(WebAssembly.validate(bytes));
const compiler = new WebAssembly.Module(bytes);
assert.deepEqual(WebAssembly.Module.imports(compiler), []);
const sha = createHash('sha256').update(bytes).digest('hex');
const api = new WebAssembly.Instance(compiler, {}).exports;
assert.equal(api.abi_version(), 2);

const tmp = await mkdtemp(join(tmpdir(), 'atto-public-candidate-'));
const staged = join(tmp, 'play');
const profile = join(tmp, 'chromium');
const downloads = join(tmp, 'downloads');
let server, browser, ws;
const requests = [];
let passed = 0;
function pass(name) { passed++; console.log('PASS CANDIDATE IDE ' + passed + ': ' + name); }
const sleep = n => new Promise(ok => setTimeout(ok,n));
async function poll(fn, label, attempts=220) {
  let last = 'not available';
  for (let i=0; i<attempts; i++) {
    try { const x = await fn(); if (x) return x; last=x; }
    catch (e) { last=e.message; }
    await sleep(80);
  }
  throw new Error(label + ' timed out: ' + String(last).slice(0,1000));
}
try {
  await mkdir(staged, {recursive:true});
  await mkdir(profile, {recursive:true});
  await mkdir(downloads, {recursive:true});
  for (const file of assets) {
    const dst=join(staged,file);
    await mkdir(join(dst,'..'), {recursive:true});
    await cp(join(publicCheckout,'play',file), dst);
  }
  // Replace only *temporary* staged bytes/pin, never a working-tree source or
  // public Git branch. Require SHA pin and receipt to agree before serving.
  await writeFile(join(staged,'public/atto_compiler.wasm'),bytes);
  const receipt = {
    schema:'atto.browser.compiler.vendor.v1',
    source_repository:'atto-lang/atto-source.rs',
    source_head:sourceHead, source_tree:sourceTree,
    toolchain:'CI-built Rust wasm32 compiler',
    target:'wasm32-unknown-unknown',
    compiler_abi:2, guest_abi:'atto.browser.guest.v1',
    compiler_sha256:sha, compiler_bytes:bytes.length
  };
  await writeFile(join(staged,'public/atto_compiler.receipt.json'),JSON.stringify(receipt,null,2)+'\n');
  await writeFile(join(staged,'src/compiler-pin.js'),
    '// Ephemeral test-only candidate pin; never committed or published.\n' +
    "export const COMPILER_SHA256 = '" + sha + "';\n" +
    'export const COMPILER_BYTES = ' + bytes.length + ';\n');
  assert.equal(createHash('sha256').update(await readFile(join(staged,'public/atto_compiler.wasm'))).digest('hex'),sha);
  assert.equal((await readdir(staged)).join(','),['docs','examples','index.html','public','src','styles.css'].join(','));
  pass('pin/source receipt and '+assets.length+'-file allowlist are internally consistent; no tests or candidate metadata staged');

  server = createServer(async(req,res)=>{
    requests.push({method:req.method,url:req.url});
    try {
      if(req.method!=='GET') throw new Error('Non-GET');
      const name=new URL(req.url,'http://localhost').pathname;
      if(!name.startsWith('/play/')) throw new Error('Non-playground request');
      const rel=decodeURIComponent(name.slice(6)) || 'index.html';
      if(!assets.includes(rel)) throw new Error('Not in release allowlist');
      const file=resolve(staged,rel);
      if(!file.startsWith(staged+sep) || !(await stat(file)).isFile()) throw new Error('Non-file');
      res.writeHead(200,{'Content-Type':mime[extname(file)]||'application/octet-stream',
        'X-Content-Type-Options':'nosniff','Cache-Control':'no-store'});
      res.end(await readFile(file));
    } catch {
      res.writeHead(404);res.end('Not found');
    }
  });
  await new Promise(ok=>server.listen(0,'127.0.0.1',ok));
  const url='http://127.0.0.1:'+server.address().port+'/play/';
  let chromeErrors='';
  browser=spawn(process.env.CHROME_BIN||'/usr/bin/google-chrome',[
    '--headless=new','--no-sandbox','--disable-dev-shm-usage','--disable-gpu',
    '--disable-background-networking','--no-first-run','--no-proxy-server',
    '--remote-debugging-port=0','--user-data-dir='+profile,'about:blank'
  ],{stdio:['ignore','ignore','pipe']});
  browser.stderr.on('data',chunk=>{chromeErrors=(chromeErrors+chunk.toString()).slice(-2000);});
  const port=await poll(async()=>{
    if(browser.exitCode!==null) throw new Error('Chromium exit: '+chromeErrors);
    try {return Number((await readFile(join(profile,'DevToolsActivePort'),'utf8')).split('\n')[0]) || null;}
    catch{return null;}
  },'Chromium DevTools port',300);
  const targets=await (await fetch('http://127.0.0.1:'+port+'/json/list')).json();
  const page=targets.find(x=>x.type==='page'&&x.webSocketDebuggerUrl);
  assert.ok(page,'Chromium page absent');
  ws=new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((yes,no)=>{
    ws.addEventListener('open',yes,{once:true});
    ws.addEventListener('error',no,{once:true});
  });
  const pending=new Map();let id=0;
  ws.addEventListener('message',event=>{
    const obj=JSON.parse(event.data);
    if(!pending.has(obj.id))return;
    const {yes,no}=pending.get(obj.id);pending.delete(obj.id);
    if(obj.error)no(new Error(obj.error.message));else yes(obj.result);
  });
  function send(method,params={}){
    return new Promise((yes,no)=>{
      const key=++id;pending.set(key,{yes,no});
      ws.send(JSON.stringify({id:key,method,params}));
    });
  }
  async function evaluate(code) {
    const result=await send('Runtime.evaluate',{expression:code,returnByValue:true,awaitPromise:true});
    if(result.exceptionDetails)throw new Error(result.exceptionDetails.text);
    return result.result?.value;
  }
  await send('Page.enable');await send('Runtime.enable');
  await send('Page.setDownloadBehavior',{behavior:'allow',downloadPath:downloads});
  await send('Page.navigate',{url});
  const view=()=>evaluate("(()=>({source:document.querySelector('#source')?.value,runDisabled:document.querySelector('#run')?.disabled,summary:document.querySelector('#summary')?.textContent,kind:document.querySelector('#summary')?.dataset.kind,output:document.querySelector('#output')?.textContent,diagnostics:document.querySelector('#diagnostics')?.textContent,bytes:document.querySelector('#bytes')?.textContent,downloadDisabled:document.querySelector('#download')?.disabled,status:document.querySelector('#status')?.textContent}))()");
  await poll(async()=>{const x=await view();return x?.source?.includes('fnc main')&&x?.status==='Ready'?x:null},'Playground UI load');
  pass('real playground loads in Chromium with the candidate compiler staged under /play/');
  const setSource=source=>evaluate("(()=>{const e=document.querySelector('#source');e.value="+JSON.stringify(source)+";e.dispatchEvent(new Event('input',{bubbles:true}));})()");
  const click=id=>evaluate("document.querySelector("+JSON.stringify('#'+id)+").click()");
  async function run(kind,needle) {
    await click('run');
    return poll(async()=>{
      const x=await view();
      return x?.kind===kind&&!x.runDisabled&&(x.output?.includes(needle)||x.diagnostics?.includes(needle))?x:null;
    },'run '+kind+' '+needle,240);
  }
  const scalar='pub fnc main(): int { return 6 * 7; }';
  await setSource(scalar); let state=await run('success','main() → 42');
  assert.equal(state.downloadDisabled,false);
  pass('real IDE Run emits and executes scalar Wasm returning 42');
  const point=[
    'define class Point as','  int x: 40','  int y: 2','  bool active: true','end',
    'pub fnc main(): int {','  val p = new Point();',
    '  if p.active then return p.x + p.y; else return 0; fi','}'
  ].join('\n');
  await setSource(point);state=await run('success','main() → 42');
  assert.match(state.bytes,/^\d+ bytes$/);
  pass('real IDE Run compiles executable Point/new constructor and fields to 42');
  await click('download');
  const downloaded=await poll(async()=>{
    const files=(await readdir(downloads)).filter(n=>n.endsWith('.wasm'));
    if(!files.length)return null;
    const blob=await readFile(join(downloads,files[0]));
    return WebAssembly.validate(blob)?blob:null;
  },'downloaded actual candidate guest');
  const inst=await WebAssembly.instantiate(downloaded,{atto:{print_i64:()=>{},print_bool:()=>{},print_str:()=>{},print_newline:()=>{}}});
  assert.equal(inst.instance.exports.main(),42n);
  // A real compiler-produced Point class uses a single fixed 64-KiB guest
  // memory. Check that the instance itself cannot grow past that bound.
  const heap=inst.instance.exports.memory;
  assert.ok(heap instanceof WebAssembly.Memory,
    'Point constructor must allocate its own bounded Wasm linear memory');
  assert.equal(heap.buffer.byteLength,65536);
  assert.throws(()=>heap.grow(1),RangeError,
    'compiled class heap unexpectedly grew above its fixed one-page limit');
  // Cross-repo ABI: the actual private Rust compiler must never emit a
  // WebAssembly start section (which runs before main and host admission).
  const {admitGuestWasm}=await import('../public-site-checkout/play/src/wasm-admission.js');
  assert.equal(admitGuestWasm(downloaded),downloaded,
    'compiler-produced guest must satisfy the hardened public Wasm admission contract');
  assert.ok(assets.includes('src/wasm-admission.js'),
    'the temporary candidate browser must stage its new shared worker import');
  pass('downloaded compiled guest Wasm independently validates and executes 42');
  await setSource(point.replace('new Point()', 'Point()'));
  state=await run('error','E_NEW_REQUIRED');
  assert.equal(state.bytes,'—');assert.equal(state.downloadDisabled,true);
  pass('bare Point() is E_NEW_REQUIRED with zero downloadable artifact');
  await setSource(point.replace('p.x + p.y','p.missing'));
  state=await run('error','E_UNKNOWN_FIELD');
  assert.equal(state.bytes,'—');assert.equal(state.downloadDisabled,true);
  pass('unknown class field rejected without executable artifact');
  const en=[
    'define enum Status as Pending, Ready end',
    'pub fnc main(): int {',
    '  val status: Status = Status.Ready;',
    '  if status eq Status.Ready then return 42; else return 0; fi','}'
  ].join('\n');
  await setSource(en);state=await run('success','main() → 42');
  pass('nominal enum variant compiles and executes in real IDE');
  await setSource(en.replaceAll('Status.Ready','Status.Unknown'));
  state=await run('error','E_UNKNOWN_VARIANT');
  assert.equal(state.bytes,'—');assert.equal(state.downloadDisabled,true);
  pass('unknown enum variant rejected with zero artifact');
  await setSource('pub fnc main(): int { return 7 % 2; }');
  state=await run('error','E_UNSUPPORTED');
  assert.equal(state.bytes,'—');assert.equal(state.downloadDisabled,true);
  pass('removed remainder operator fails closed, rather than runtime division-by-zero');
  const actorProgram=[
    'define actor Worker as',
    '  receive(msg: int) -> int { return msg + 1; }',
    'end',
    'pub fnc main(): void {',
    '  val first = spawn Worker();',
    '  val second = spawn Worker();',
    '  val a = first.send(41);',
    '  val b = second.send(41);',
    '  await first.get_done();',
    '}',
  ].join('\n');
  await setSource(actorProgram);
  state=await run('success','main() → 0');
  assert.match(state.output,
    /actor\[1\] receive\(41\) -> 42\nactor\[2\] receive\(41\) -> 42\nmain\(\) → 0/);
  assert.ok(requests.some(x=>x.url.includes('/play/src/actor-guest-worker.js')),
    'nested actor worker script was fetched by a real browser');
  pass('Atto source compiles and executes two independently owned Web Workers with acknowledged completion');
  await setSource(actorProgram.replace('return msg + 1;','stdio.println(msg); return msg + 1;'));
  state=await run('error','E_UNSUPPORTED');
  assert.equal(state.bytes,'—');assert.equal(state.downloadDisabled,true);
  pass('actor receive cannot gain stdout capability; rejected compile emits no Wasm artifact');
  await setSource(point);state=await run('success','main() → 42');
  pass('worker recovery after multiple negative compile cases');
  await setSource('pub fnc main(): int { loop { } }');
  state=await run('error','deadline');
  pass('infinite guest execution stopped by the browser worker deadline');
  await setSource(point);state=await run('success','main() → 42');
  pass('recover and recompile class program after cancelled worker');
  assert.ok(requests.some(x=>x.url.includes('/play/public/atto_compiler.wasm?sha256='+sha.slice(0,16))));
  assert.ok(requests.every(x=>x.method==='GET'),'Source code submitted to server');
  assert.ok(!requests.some(x=>/\/(tests|release-candidates)\//.test(x.url)));
  pass('candidate URL cache pin is exact; all browser requests GET; no private metadata requested');
  console.log('PASS: '+passed+' real Chromium candidate-enabled PUBLIC-FRONTEND scenarios, source='+sourceHead+
    ', public_frontend='+PUBLIC_SITE_COMMIT+', compiler_sha256='+sha);
  console.log('PUBLIC_RELEASE=NOT_AUTHORIZED; LIVE_DEPLOYMENT=NOT_EXECUTED; PINNED_PUBLIC_COMPILER=NOT_MODIFIED');
} finally {
  try { ws?.close(); } catch {}
  if(browser?.exitCode===null) {
    browser.kill('SIGKILL');
    await sleep(150);
  }
  if(server) await new Promise(ok=>server.close(ok));
  await rm(tmp,{recursive:true,force:true,maxRetries:5,retryDelay:100});
}
