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
  ['/channel-host-probe.mjs',[new URL('./browser-async/channel-host-probe.mjs',import.meta.url),'text/javascript; charset=utf-8']],
  ['/channel-int-v1.mjs',[new URL('../runtime/browser/channel_int_v1.mjs',import.meta.url),'text/javascript; charset=utf-8']],
  ['/channel-guest-abi-probe.mjs',[new URL('./browser-async/channel-guest-abi-probe.mjs',import.meta.url),'text/javascript; charset=utf-8']],
  ['/runtime/browser/channel_int_guest_v1.mjs',[new URL('../runtime/browser/channel_int_guest_v1.mjs',import.meta.url),'text/javascript; charset=utf-8']],
  ['/runtime/browser/channel_int_v1.mjs',[new URL('../runtime/browser/channel_int_v1.mjs',import.meta.url),'text/javascript; charset=utf-8']],
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
  // Test the actual Worker host channel runtime independently of guest
  // compilation. This must NEVER count as the missing Channel<T> guest ABI.
  const hostProbe=await evaluate(`new Promise((resolve,reject)=>{
    const worker=new Worker('/channel-host-probe.mjs',{type:'module'});
    const timeout=setTimeout(()=>{worker.terminate();reject(new Error('channel host timeout'))},5000);
    worker.onmessage=e=>{clearTimeout(timeout);worker.terminate();resolve(e.data)};
    worker.onerror=e=>{clearTimeout(timeout);worker.terminate();reject(new Error('channel host worker error'))};
    worker.postMessage({});
  })`);
  assert.deepEqual(hostProbe,{
    ok:true,answer:'42',singleConsumption:'AlreadyConsumed',
    cancellation:'Cancelled',eof:true,sendAfterClose:'Closed'
  });
  console.log('PASS browser host: Channel<int> checked Future, cancellation and EOF in actual Worker (NOT guest ABI)');
  // Real Wasm import/worker integration, distinct from compiler-emitted reads.
  const abiProbe=await evaluate("new Promise((resolve,reject)=>{"+
    "const w=new Worker('/channel-guest-abi-probe.mjs',{type:'module'});"+
    "const t=setTimeout(()=>{w.terminate();reject(Error('channel ABI timeout'))},5000);"+
    "w.onmessage=e=>{clearTimeout(t);w.terminate();resolve(e.data)};"+
    "w.onerror=e=>{clearTimeout(t);w.terminate();reject(Error('channel ABI Worker error'))};"+
    "w.postMessage({});})");
  assert.deepEqual(abiProbe,{ok:true,importExecuted:true,some:41,eof:true,
    replay:'AlreadyConsumed',released:true});
  console.log('PASS channel ABI: genuine Wasm import and tagged Future outcome in Chromium Worker (NOT AttoLang guest lowering)');


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
  const channelGuest="pub susp fnc main(): int {\n  val opened: Result<Channel<int>,IoError> = Channel<int>.bounded(0);\n  ^ match opened over\n    on Err(error) -> { return -3; }\n    on Ok(channel) -> {\n      val pending: Future<Result<Option<int>,IoError>> = nb readch channel;\n      val sent: Result<void,IoError> = channel.send(41);\n      ^ match sent over\n        on Err(error) -> { return -2; }\n        on Ok -> {\n          val read_result: Result<Option<int>,IoError> = await pending;\n          ^ match read_result over\n            on Err(error) -> { return -1; }\n            on Ok(item) -> {\n              ^ match item over\n                on None -> { return 0; }\n                on Some(value) -> { return value + 1; }\n              end\n            }\n          end\n        }\n      end\n    }\n  end\n}";
  assert.match(await run(channelGuest,'main() -> 42'),/main\(\) -> 42/);
  assert.equal(await evaluate('window.__attoLastCooperativeYields'),1,
    'channel read await must yield cooperatively once');
  console.log('PASS compiled AttoLang Channel<int> read Future, Some(41) -> 42');

  const channelAbandonSource=`pub susp fnc main(): int {
    val opened = Channel<int>.bounded(0);
    match opened over
      on Err(error) -> { return -1; }
      on Ok(channel) -> {
        val pending = nb readch channel;
        val sent = channel.send(41);
        match sent over
          on Err(error) -> { return -2; }
          on Ok -> {
            val abandoned: Result<void,IoError> = pending.abandon();
            match abandoned over
              on Err(error) -> { return -3; }
              on Ok -> {
                val read_again = nb readch channel;
                val outcome: Result<Option<int>,IoError> = await read_again;
                match outcome over
                  on Err(error) -> { return -4; }
                  on Ok(item) -> {
                    match item over
                      on None -> { return -5; }
                      on Some(n) -> { return n + 1; }
                    end
                  }
                end
              }
            end
          }
        end
      }
    end
  }`;
  assert.match(await run(channelAbandonSource,'main() -> 42'),/^main\(\) -> 42$/);
  assert.equal(await evaluate('window.__attoLastCooperativeYields'),1,
    'abandoned committed value must be read once, with one cooperative await');
  console.log('PASS compiled guest: pending.abandon() consumes ticket and rehomes committed value');

  // One compiled guest exercises both 64-bit extrema and two independent
  // pending readers. The return value verifies FIFO *ordering*, not just
  // a commutative checksum, and confirms no sentinel/precision truncation.
  const channelFifoI64=`pub susp fnc main(): int {
    val opened = Channel<int>.bounded(0);
    match opened over
      on Err(error) -> { return -1; }
      on Ok(channel) -> {
        val first = nb readch channel;
        val second = nb readch channel;
        val upper: int = 9223372036854775807;
        val lower: int = -9223372036854775807 - 1;
        val sent_upper = channel.send(upper);
        val sent_lower = channel.send(lower);
        val first_result = await first;
        val second_result = await second;
        match first_result over
          on Err(error) -> { return -2; }
          on Ok(first_option) -> {
            match first_option over
              on None -> { return -3; }
              on Some(a) -> {
                match second_result over
                  on Err(error) -> { return -4; }
                  on Ok(second_option) -> {
                    match second_option over
                      on None -> { return -5; }
                      on Some(b) -> {
                        if a == upper then
                          if b == lower then
                            return 42;
                          fi
                        fi
                        return -6;
                      }
                    end
                  }
                end
              }
            end
          }
        end
      }
    end
  }`;
  assert.equal((await run(channelFifoI64,'main() -> 42')).trim(),'main() -> 42');
  assert.equal(await evaluate('window.__attoLastCooperativeYields'),2,
    'each of two pending channel Futures must cooperate once');
  console.log('PASS compiled guest: two-reader FIFO preserves both signed i64 extrema');




  // Distinct compiled guest outcomes: EOF != Some(0); ordinary I/O errors
  // are tagged Results, and explicit release succeeds after draining.
  const channelEofSource="pub susp fnc main(): int {\n  val opened: Result<Channel<int>,IoError> = Channel<int>.bounded(0);\n  ^ match opened over\n    on Err(error) -> { return -10; }\n    on Ok(channel) -> {\n      val closed: Result<void,IoError> = channel.close();\n      val pending: Future<Result<Option<int>,IoError>> = nb readch channel;\n      val outcome: Result<Option<int>,IoError> = await pending;\n      val released: Result<void,IoError> = channel.release();\n      ^ match released over\n        on Err(error) -> { return -11; }\n        on Ok -> {\n          ^ match outcome over\n            on Err(error) -> { return -12; }\n            on Ok(item) -> {\n              ^ match item over\n                on None -> { return 42; }\n                on Some(value) -> { return -13; }\n              end\n            }\n          end\n        }\n      end\n    }\n  end\n}";
  assert.match(await run(channelEofSource,'main() -> 42'),/^main\(\) -> 42$/);
  assert.equal(await evaluate('window.__attoLastCooperativeYields'),1);
  console.log('PASS compiled guest Channel<int>: EOF is None, release succeeds');
  const channelClosedSource="pub fnc main(): int {\n  val opened: Result<Channel<int>,IoError> = Channel<int>.bounded(0);\n  ^ match opened over\n    on Err(error) -> { return -10; }\n    on Ok(channel) -> {\n      val closed: Result<void,IoError> = channel.close();\n      val sent: Result<void,IoError> = channel.send(41);\n      val released: Result<void,IoError> = channel.release();\n      ^ match released over\n        on Err(error) -> { return -11; }\n        on Ok -> {\n          ^ match sent over\n            on Err(error) -> { return 42; }\n            on Ok -> { return -12; }\n          end\n        }\n      end\n    }\n  end\n}";
  assert.match(await run(channelClosedSource,'main() -> 42'),/^main\(\) -> 42$/);
  assert.equal(await evaluate('window.__attoLastCooperativeYields'),0);
  console.log('PASS compiled guest Channel<int>: send-after-close is Err, release succeeds');
  const channelFullSource="pub susp fnc main(): int {\n  val opened: Result<Channel<int>,IoError> = Channel<int>.bounded(1);\n  ^ match opened over\n    on Err(error) -> { return -10; }\n    on Ok(channel) -> {\n      val first: Result<void,IoError> = channel.send(41);\n      val overflow: Result<void,IoError> = channel.send(99);\n      val pending: Future<Result<Option<int>,IoError>> = nb readch channel;\n      val outcome: Result<Option<int>,IoError> = await pending;\n      val closed: Result<void,IoError> = channel.close();\n      val released: Result<void,IoError> = channel.release();\n      ^ match released over\n        on Err(error) -> { return -11; }\n        on Ok -> {\n          ^ match overflow over\n            on Ok -> { return -12; }\n            on Err(error) -> {\n              ^ match outcome over\n                on Err(error) -> { return -13; }\n                on Ok(item) -> {\n                  ^ match item over\n                    on None -> { return -14; }\n                    on Some(value) -> { return value + 1; }\n                  end\n                }\n              end\n            }\n          end\n        }\n      end\n    }\n  end\n}";
  assert.match(await run(channelFullSource,'main() -> 42'),/^main\(\) -> 42$/);
  assert.equal(await evaluate('window.__attoLastCooperativeYields'),1);
  console.log('PASS compiled guest Channel<int>: Full Err preserves queued i64 and release succeeds');
  const channelCancelled="pub susp fnc main(): int {\n  val opened: Result<Channel<int>,IoError> = Channel<int>.bounded(0);\n  ^ match opened over\n    on Err(error) -> { return -10; }\n    on Ok(channel) -> {\n      val pending: Future<Result<Option<int>,IoError>> = nb readch channel;\n      val cancelled: Result<void,IoError> = pending.cancel();\n      val before_await: Result<void,IoError> = channel.close();\n      val still_busy: Result<void,IoError> = channel.release();\n      val resolved: Result<Option<int>,IoError> = await pending;\n      val released: Result<void,IoError> = channel.release();\n      ^ match still_busy over\n        on Ok -> { return -11; }\n        on Err(error) -> {\n          ^ match cancelled over\n            on Err(error) -> { return -12; }\n            on Ok -> {\n              ^ match released over\n                on Err(error) -> { return -13; }\n                on Ok -> {\n                  ^ match resolved over\n                    on Ok(item) -> { return -14; }\n                    on Err(error) -> { return 42; }\n                  end\n                }\n              end\n            }\n          end\n        }\n      end\n    }\n  end\n}";
  assert.match(await run(channelCancelled,'main() -> 42'),/^main\(\) -> 42$/);
  assert.equal(await evaluate('window.__attoLastCooperativeYields'),1);
  console.log('PASS compiled guest Channel<int>: cancel wins, Busy until Future awaited, then release succeeds');
  const channelCancelAfterSend="pub susp fnc main(): int {\n  val opened: Result<Channel<int>,IoError> = Channel<int>.bounded(0);\n  ^ match opened over\n    on Err(error) -> { return -10; }\n    on Ok(channel) -> {\n      val pending = nb readch channel;\n      val sent: Result<void,IoError> = channel.send(41);\n      val cancel_after_send: Result<void,IoError> = pending.cancel();\n      val received: Result<Option<int>,IoError> = await pending;\n      val closed = channel.close();\n      val released = channel.release();\n      ^ match cancel_after_send over\n        on Ok -> { return -11; }\n        on Err(error) -> {\n          ^ match released over\n            on Err(error) -> { return -12; }\n            on Ok -> {\n              ^ match received over\n                on Err(error) -> { return -13; }\n                on Ok(item) -> {\n                  ^ match item over\n                    on None -> { return -14; }\n                    on Some(value) -> { return value + 1; }\n                  end\n                }\n              end\n            }\n          end\n        }\n      end\n    }\n  end\n}";
  assert.match(await run(channelCancelAfterSend,'main() -> 42'),/^main\(\) -> 42$/);
  assert.equal(await evaluate('window.__attoLastCooperativeYields'),1);
  console.log('PASS compiled guest Channel<int>: send-before-cancel preserves Some(41) and returns AlreadySettled');

  // These execute actual compiler-emitted guest Wasm; release must succeed
  // before Worker teardown so the test detects missing lexical cleanup.
  const lexicalCleanup = `pub susp fnc main(): int {
    val opened = Channel<int>.bounded(0);
    match opened over
      on Err(error) -> { return -1; }
      on Ok(channel) -> {
        { val pending = nb readch channel; }
        val closed = channel.close();
        val released = channel.release();
        match released over
          on Err(error) -> { return -2; }
          on Ok -> { return 42; }
        end
      }
    end
  }`;
  assert.match(await run(lexicalCleanup,'main() -> 42'),/main\\(\\) -> 42/);
  console.log('PASS compiled guest: lexical Future cleanup releases channel before teardown');

  const rehomeCleanup = `pub susp fnc main(): int {
    val opened = Channel<int>.bounded(0);
    match opened over
      on Err(error) -> { return -1; }
      on Ok(channel) -> {
        {
          val pending = nb readch channel;
          val sent = channel.send(41);
        }
        val second = nb readch channel;
        val outcome = await second;
        match outcome over
          on Err(error) -> { return -2; }
          on Ok(item) -> {
            match item over
              on None -> { return -3; }
              on Some(value) -> { return value + 1; }
            end
          }
        end
      }
    end
  }`;
  assert.match(await run(rehomeCleanup,'main() -> 42'),/main\\(\\) -> 42/);
  console.log('PASS compiled guest: auto drop rehomes committed Some(41) without data loss');

  assert.match(await run('pub fnc main(): int { val pending = nb readch 41; return 0; }','E_EFFECT'),/E_EFFECT/);
  console.log('PASS compiled guest negative: nb readch without susp fails closed');
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
  const combined='pub suspends fnc main(): int { val a=task { return 42; }; stdio.out << "direct"; return await a; }';
  assert.match(await run(combined,'main() -> 42'),/^directmain\(\) -> 42$/);
  assert.equal(await evaluate('window.__attoLastCooperativeYields'),1,'combined stdout/Future guest must yield exactly once');
  console.log('PASS integration: direct stdio.out << and awaited Future<int> execute together inside Chromium Worker');

  // Compile the new pipeline in the actual wasm32 compiler, execute its
  // guest Wasm in Chromium Worker, and prove the existing JSPI await
  // and cooperative scheduler path is used (not a host JS mock).
  const piped='fnc transform_result(int v): int { return v + 1; } pub suspends fnc main(): int { val future=task { return 41; }; return future |> suspends |v| -> await v |> transform_result; }';
  assert.match(await run(piped,'main() -> 42'),/^main\(\) -> 42$/);
  assert.equal(await evaluate('window.__attoLastCooperativeYields'),1,
    'the explicit suspends lambda stage must cooperatively await exactly once');
  console.log('PASS integration: explicit suspends identity-await stage executes real guest Future<int> and named transform');

  const captured='fnc transform_result(int x): int { return x + 1; } pub suspends fnc main(): int { val future=task { return 41; }; future |> susp |f| -> await f |> transform_result >> val result: int; stdio.println(result); return result; }';
  assert.match(await run(captured,'main() -> 42'),/^42\nmain\(\) -> 42$/);
  assert.equal(await evaluate('window.__attoLastCooperativeYields'),1,
    'capturing the transformed Future result must await once, not register another read');
  console.log('PASS integration: terminal >> val result captures a real JSPI awaited Future<int>');

  // Exact multiline caret-return terminal capture requested in Atto syntax:
  // caret returns the checked value bound by >> val; there is no second await.
  const caretCapture=`fnc transform_result(int v): int { return v + 1; }
pub suspends fnc main(): int {
    val pending = task { return 41; };

    ^ pending
        |> susp |f| -> await f
        |> transform_result
        >> val result: int;
}`;
  assert.match(await run(caretCapture,'main() -> 42'),/^main\(\) -> 42$/);
  assert.equal(await evaluate('window.__attoLastCooperativeYields'),1,
    'caret await-transform-capture must suspend exactly once');
  console.log('PASS integration: exact multiline caret-return pipeline compiles and runs as a guest returning 42');

  const captureDirect='pub suspends fnc main(): int { val pending=task { return 42; }; pending |> suspends |p| -> await p >> val answer: int; return answer; }';
  assert.match(await run(captureDirect,'main() -> 42'),/^main\(\) -> 42$/);
  assert.equal(await evaluate('window.__attoLastCooperativeYields'),1,
    'direct terminal capture must yield once');
  console.log('PASS integration: direct terminal await capture executes in browser guest');

  assert.match(await run(
    'pub suspends fnc main(): int { val pending=task {return 42;}; pending |> susp |p| -> await p >> val answer: string; return 42; }',
    'E_TYPE'),/E_TYPE/);
  console.log('PASS integration: mismatched terminal capture rejects without executable guest');

  for (const source of [
    'pub suspends fnc main(): int { nb stdio.in >> (val String line << await Future<string>); return 42; }',
    'pub suspends fnc main(): int { nb stdio.in >> Future<string> x | await x >> val String line; return 42; }',
  ]) {
    assert.match(await run(source,'E_UNSUPPORTED'),/E_UNSUPPORTED/);
  }
  console.log('PASS integration: both contextual nb stdin read-await spellings fail closed until guest I/O ABI');

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
