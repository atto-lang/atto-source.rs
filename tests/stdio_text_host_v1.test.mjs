import test from 'node:test';
import assert from 'node:assert/strict';
import {StdioTextHostV1} from '../runtime/browser/stdio_text_host_v1.mjs';

const encode = text => new TextEncoder().encode(text);
async function* chunks(...items) {for (const item of items) yield item instanceof Uint8Array?item:encode(item)}
const mk = (stdin,opts={}) => {
  const out=[],error=[];
  const host=new StdioTextHostV1({tenantId:'test-tenant',generation:1,stdin,stdout:async b=>out.push(new TextDecoder().decode(b)),
    stderr:async b=>error.push(new TextDecoder().decode(b)),...opts});
  return {host,out,error};
};

test('explicit injection, identity aliases, typed directions, exact echoed bytes',async()=>{
  assert.throws(()=>new StdioTextHostV1(), /generation/);
  assert.throws(()=>new StdioTextHostV1({generation:1,tenantId:'test-tenant'}), /stdin source/);
  const {host,out,error}=mk(chunks('hello\n'));
  assert.equal(host.stdio.in,host.process.stdin);
  assert.equal(host.stdio.out,host.process.stdout);
  assert.equal(host.stdio.err,host.process.stderr);
  assert.notEqual(host.stdio.out,host.stdio.err);
  assert.equal(typeof host.stdio.in.writech,'undefined');
  assert.equal(typeof host.stdio.out.readch,'undefined');
  assert.deepEqual(await host.stdio.out.writech('Echo the next input item:'),{ok:true,value:undefined});
  assert.deepEqual(await host.stdio.in.readch(),{ok:true,value:{some:true,value:'hello'}});
  assert.deepEqual(await host.stdio.out.writech('hello'),{ok:true,value:undefined});
  assert.deepEqual(await host.stdio.err.writech('echo complete'),{ok:true,value:undefined});
  assert.deepEqual(await host.stdio.in.readch(),{ok:true,value:{some:false}});
  assert.deepEqual(out,['Echo the next input item:','hello']);
  assert.deepEqual(error,['echo complete']);
});

test('LF, CRLF, empty record, final unterminated input, Unicode, NUL and chunk-split UTF-8',async()=>{
  const item=encode('é💡');
  const {host}=mk(chunks('first\r\n\n',item.slice(0,3),item.slice(3),'\n','a\0b'));
  const read=()=>host.stdio.in.readch();
  assert.deepEqual(await read(),{ok:true,value:{some:true,value:'first'}});
  assert.deepEqual(await read(),{ok:true,value:{some:true,value:''}});
  assert.deepEqual(await read(),{ok:true,value:{some:true,value:'é💡'}});
  assert.deepEqual(await read(),{ok:true,value:{some:true,value:'a\0b'}});
  assert.deepEqual(await read(),{ok:true,value:{some:false}});
  assert.deepEqual(await read(),{ok:true,value:{some:false}});
});

test('malformed UTF-8 and input quotas report errors rather than truncation',async()=>{
  const invalid=mk(chunks(new Uint8Array([0xc3,0x28,10])));
  assert.deepEqual(await invalid.host.stdio.in.readch(),{ok:false,error:{kind:'MalformedUtf8'}});
  const oversized=mk(chunks('abcde\n'),{maxRecordBytes:4});
  assert.deepEqual(await oversized.host.stdio.in.readch(),{ok:false,error:{kind:'RecordTooLarge'}});
  const q=mk(chunks('\n\n\n'),{maxQueuedRecords:2});
  // Already delimited records can be delivered without silently discarding EOF;
  // subsequent errors remain visible if the host violates bounded queue caps.
  assert.deepEqual(await q.host.stdio.in.readch(),{ok:true,value:{some:true,value:''}});
  assert.deepEqual(await q.host.stdio.in.readch(),{ok:true,value:{some:true,value:''}});
  assert.deepEqual(await q.host.stdio.in.readch(),{ok:false,error:{kind:'Quota'}});
  const badChunks=mk(chunks('abcde'),{maxChunkBytes:3});
  assert.deepEqual(await badChunks.host.stdio.in.readch(),{ok:false,error:{kind:'Quota'}});
});

test('stream() has exclusive lease across process.stdin and stdio.in',async()=>{
  const {host}=mk(chunks('a\nb\nc\n'));
  const stream=host.stdio.in.stream();
  assert.deepEqual(await host.process.stdin.readch(),{ok:false,error:{kind:'ReaderBusy'}});
  assert.throws(()=>host.process.stdin.stream(),/ReaderBusy/);
  assert.deepEqual(await stream.next(),{done:false,value:'a'});
  assert.deepEqual(await stream.return(),{done:true});
  assert.deepEqual(await host.stdio.in.readch(),{ok:true,value:{some:true,value:'b'}});
  assert.deepEqual(await host.stdio.in.readch(),{ok:true,value:{some:true,value:'c'}});
  assert.deepEqual(await host.stdio.in.readch(),{ok:true,value:{some:false}});
});

test('abort before input commit detaches waiter without losing next record',async()=>{
  let deliver;
  async function* input() {yield await new Promise(resolve=>{deliver=resolve})}
  const {host}=mk(input());
  const abort=new AbortController();
  const waiting=host.stdio.in.readch({signal:abort.signal});
  assert.deepEqual(await host.stdio.in.readch(),{ok:false,error:{kind:'ReaderBusy'}});
  abort.abort();
  assert.deepEqual(await waiting,{ok:false,error:{kind:'Cancelled'}});
  const next=host.stdio.in.readch();
  deliver(encode('saved\n'));
  assert.deepEqual(await next,{ok:true,value:{some:true,value:'saved'}});
  assert.deepEqual(await host.stdio.in.readch(),{ok:true,value:{some:false}});
});

test('revocation denies pending readers and all future host-port access',async()=>{
  let deliver;
  async function* input(){yield await new Promise(resolve=>{deliver=resolve})}
  const {host,out}=mk(input());
  const waiting=host.stdio.in.readch();
  host.revoke();
  assert.deepEqual(await waiting,{ok:false,error:{kind:'Revoked'}});
  assert.deepEqual(await host.process.stdin.readch(),{ok:false,error:{kind:'Revoked'}});
  assert.throws(()=>host.stdio.in.stream(),/Revoked/);
  assert.deepEqual(await host.stdio.out.writech('denied'),{ok:false,error:{kind:'Revoked'}});
  assert.deepEqual(out,[]);
  deliver(encode('stale\n'));
  await Promise.resolve();
});

test('output serializes bounded writes, rejects quota and retains byte order',async()=>{
  let firstRelease;const written=[];
  let calls=0;
  const writer=async b=>{
    const text=new TextDecoder().decode(b);
    written.push(text);calls++;
    if(calls===1)await new Promise(resolve=>{firstRelease=resolve});
  };
  const {host}=mk(chunks(),{stdout:writer,maxPendingWrites:2,maxPendingBytes:16});
  const a=host.stdio.out.writech('one');
  const b=host.stdio.out.writech('two');
  assert.deepEqual(await host.stdio.out.writech('three'),{ok:false,error:{kind:'Full'}});
  assert.deepEqual(written,['one']);
  firstRelease();
  assert.deepEqual(await a,{ok:true,value:undefined});
  assert.deepEqual(await b,{ok:true,value:undefined});
  assert.deepEqual(written,['one','two']);
  assert.deepEqual(await host.stdio.out.writech(1),{ok:false,error:{kind:'InvalidValue'}});
});

test('sink failure is checked, not a silent success, and terminal',async()=>{
  const {host}=mk(chunks(),{stdout:async()=>{throw Error('EPIPE')}});
  assert.deepEqual(await host.stdio.out.writech('oops'),{ok:false,error:{kind:'SinkFailure'}});
  assert.deepEqual(await host.stdio.out.writech('again'),{ok:false,error:{kind:'SinkFailure'}});
});

test('generation and quota configuration fail closed before acquiring input',()=>{
  assert.throws(()=>mk(chunks(),{tenantId:''}),/tenant identity/);
  assert.throws(()=>mk(chunks(),{tenantId:'   '}),/tenant identity/);
  assert.throws(()=>mk(chunks(),{generation:0}),/generation/);
  assert.throws(()=>mk(chunks(),{generation:-1}),/generation/);
  assert.throws(()=>mk(chunks(),{generation:1.25}),/generation/);
  assert.throws(()=>mk(chunks(),{maxRecordBytes:Infinity}),/quota/);
  assert.throws(()=>mk(chunks(),{maxPendingBytes:0}),/quota/);
});

test('revocation rejects queued writes but retains an already-admitted write outcome',async()=>{
  let complete;
  const written=[];
  const sink=async bytes=>{
    written.push(new TextDecoder().decode(bytes));
    await new Promise(resolve=>{complete=resolve});
  };
  const {host}=mk(chunks(),{stdout:sink});
  const admitted=host.stdio.out.writech('admitted');
  const waiting=host.stdio.out.writech('queued');
  host.revoke();
  assert.deepEqual(await waiting,{ok:false,error:{kind:'Revoked'}});
  complete();
  assert.deepEqual(await admitted,{ok:true,value:undefined});
  assert.deepEqual(await host.stdio.out.writech('denied'),{ok:false,error:{kind:'Revoked'}});
  assert.deepEqual(written,['admitted']);
});
