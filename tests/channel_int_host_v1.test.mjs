import test from 'node:test';
import assert from 'node:assert/strict';
import { ChannelIntHostV1 } from '../runtime/browser/channel_int_v1.mjs';

const fresh=(capacity=1)=>{const rt=new ChannelIntHostV1();const opened=rt.open(capacity);assert.equal(opened.ok,true);return {rt,ch:opened.value}};
const item=n=>({ok:true,value:{some:true,value:BigInt(n)}});
const eof={ok:true,value:{some:false}};
const error=kind=>({ok:false,error:{kind}});

test('immediately registered pending Future resumes on first send, once only',async()=>{
  const {rt,ch}=fresh();const pending=rt.nb_read(ch).value;
  assert.deepEqual(rt.snapshot(ch).value.pending,1);
  const promise=rt.await_read(pending);
  assert.deepEqual(await rt.await_read(pending),error('AlreadyConsumed'));
  assert.deepEqual(rt.send(ch,41n),{ok:true,value:{admitted:true}});
  assert.deepEqual(await promise,item(41));
  assert.deepEqual(await rt.await_read(pending),error('AlreadyConsumed'));
  assert.equal(rt.snapshot(ch).value.pending,0);
});

test('FIFO across two independent pending reads and no duplicate delivery',async()=>{
  const {rt,ch}=fresh(2);
  const a=rt.nb_read(ch).value,b=rt.nb_read(ch).value;
  const pa=rt.await_read(a),pb=rt.await_read(b);
  rt.send(ch,11n);rt.send(ch,22n);
  assert.deepEqual(await pa,item(11));
  assert.deepEqual(await pb,item(22));
  assert.deepEqual(rt.snapshot(ch).value,{queued:0,pending:0,closed:false,capacity:2,outstanding:0});
});

test('bounded capacity refuses speculative admission without consuming buffered item',async()=>{
  const {rt,ch}=fresh(1);
  assert.equal(rt.send(ch,41n).ok,true);
  assert.deepEqual(rt.send(ch,99n),error('Full'));
  assert.deepEqual(rt.snapshot(ch).value.queued,1);
  const a=rt.nb_read(ch).value;
  assert.deepEqual(await rt.await_read(a),item(41));
  assert.equal(rt.send(ch,42n).ok,true);
  assert.deepEqual(await rt.await_read(rt.nb_read(ch).value),item(42));
});

test('rendezvous capacity zero needs pending registration before send',async()=>{
  const {rt,ch}=fresh(0);
  assert.deepEqual(rt.send(ch,3n),error('Full'));
  const f=rt.nb_read(ch).value;
  assert.equal(rt.send(ch,3n).ok,true);
  assert.deepEqual(await rt.await_read(f),item(3));
});

test('EOF is absence, not an implicit zero message, and close never closes output',async()=>{
  const {rt,ch}=fresh(2);
  const f=rt.nb_read(ch).value;
  rt.close(ch);
  assert.deepEqual(await rt.await_read(f),eof);
  assert.deepEqual(await rt.await_read(rt.nb_read(ch).value),eof);
  assert.deepEqual(rt.send(ch,0n),error('Closed'));
  assert.deepEqual(rt.close(ch),{ok:true,value:{alreadyClosed:true}});
});

test('queued i64 payloads survive closing before eventual EOF',async()=>{
  const {rt,ch}=fresh(2);
  rt.send(ch,0n);rt.send(ch,-1n);rt.close(ch);
  assert.deepEqual(await rt.await_read(rt.nb_read(ch).value),item(0));
  assert.deepEqual(await rt.await_read(rt.nb_read(ch).value),item(-1));
  assert.deepEqual(await rt.await_read(rt.nb_read(ch).value),eof);
});

test('cancellation removes only that registration, preserving next waiter and value',async()=>{
  const {rt,ch}=fresh(1);
  const a=rt.nb_read(ch).value,b=rt.nb_read(ch).value;
  const abandoned=rt.await_read(a),next=rt.await_read(b);
  assert.deepEqual(rt.cancel(a),{ok:true,value:{cancelled:true}});
  rt.send(ch,42n);
  assert.deepEqual(await abandoned,error('Cancelled'));
  assert.deepEqual(await next,item(42));
  assert.deepEqual(rt.cancel(a),error('AlreadyConsumed'));
});

test('commit linearizes before later cancellation: value remains owned and awaitable',async()=>{
  const {rt,ch}=fresh();
  const f=rt.nb_read(ch).value;
  rt.send(ch,123n);
  assert.deepEqual(rt.cancel(f),error('AlreadySettled'));
  assert.deepEqual(await rt.await_read(f),item(123));
});

test('cancel linearizes first: later send is never delivered to cancelled registration',async()=>{
  const {rt,ch}=fresh(1);
  const f=rt.nb_read(ch).value;
  assert.equal(rt.cancel(f).ok,true);
  assert.equal(rt.send(ch,11n).ok,true);
  assert.deepEqual(await rt.await_read(f),error('Cancelled'));
  assert.deepEqual(await rt.await_read(rt.nb_read(ch).value),item(11));
});

test('close linearizes before later cancellation, with no erased EOF',async()=>{
  const {rt,ch}=fresh();
  const f=rt.nb_read(ch).value;
  rt.close(ch);
  assert.deepEqual(rt.cancel(f),error('AlreadySettled'));
  assert.deepEqual(await rt.await_read(f),eof);
});

test('strict i64 payload validation preserves signed 64-bit edges',async()=>{
  const {rt,ch}=fresh(2);
  const lo=-(1n<<63n), hi=(1n<<63n)-1n;
  rt.send(ch,lo);rt.send(ch,hi);
  assert.deepEqual(await rt.await_read(rt.nb_read(ch).value),item(lo));
  assert.deepEqual(await rt.await_read(rt.nb_read(ch).value),item(hi));
  for(const invalid of [0,1.0,NaN,'42',1n<<63n,-(1n<<63n)-1n]) {
    assert.deepEqual(rt.send(ch,invalid),error('InvalidValue'));
  }
});

test('pending-read quota errors are checked and recover after settlement',async()=>{
  const rt=new ChannelIntHostV1({maxChannels:2,maxTickets:2,maxCapacity:2});
  const ch=rt.open(0).value;
  const a=rt.nb_read(ch).value,b=rt.nb_read(ch).value;
  assert.deepEqual(rt.nb_read(ch),error('Quota'));
  rt.cancel(a);
  assert.deepEqual(await rt.await_read(a),error('Cancelled'));
  const c=rt.nb_read(ch).value;
  rt.send(ch,4n);rt.send(ch,5n);
  assert.deepEqual(await rt.await_read(b),item(4));
  assert.deepEqual(await rt.await_read(c),item(5));
});

test('capacity and channel quota errors cannot create invalid handles',()=>{
  const rt=new ChannelIntHostV1({maxChannels:1,maxTickets:1,maxCapacity:1});
  assert.deepEqual(rt.open(-1),error('InvalidCapacity'));
  assert.deepEqual(rt.open(2),error('InvalidCapacity'));
  const a=rt.open(0);assert.equal(a.ok,true);
  assert.deepEqual(rt.open(1),error('Quota'));
  assert.deepEqual(rt.nb_read(987654),error('InvalidHandle'));
  assert.deepEqual(rt.send(987654,1n),error('InvalidHandle'));
  assert.deepEqual(rt.close(987654),error('InvalidHandle'));
});

test('one channel cannot steal the other channel pending values',async()=>{
  const rt=new ChannelIntHostV1();
  const a=rt.open(1).value,b=rt.open(1).value;
  const fa=rt.nb_read(a).value,fb=rt.nb_read(b).value;
  rt.send(b,42n);
  assert.deepEqual(await rt.await_read(fb),item(42));
  assert.equal(rt.snapshot(a).value.pending,1);
  rt.send(a,41n);
  assert.deepEqual(await rt.await_read(fa),item(41));
});

test('invalid Future tickets fail without hanging, even after retiring one',async()=>{
  const {rt,ch}=fresh();
  assert.deepEqual(await rt.await_read(0),error('InvalidHandle'));
  assert.deepEqual(await rt.await_read(999999),error('InvalidHandle'));
  const id=rt.nb_read(ch).value;rt.cancel(id);
  assert.deepEqual(await rt.await_read(id),error('Cancelled'));
  assert.deepEqual(await rt.await_read(id),error('AlreadyConsumed'));
});

test('delayed read resumes through Worker event loop and preserves checked result',async()=>{
  const {rt,ch}=fresh();
  const ticket=rt.nb_read(ch).value;
  const result=rt.await_read(ticket);
  setImmediate(()=>rt.send(ch,41n));
  const settled=await result;
  assert.deepEqual(settled,item(41));
  assert.equal(settled.value.value+1n,42n);
});
