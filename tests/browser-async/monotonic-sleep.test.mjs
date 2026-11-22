// Actual Worker helper unit tests with an injected monotonic fake clock.
// These are host-adapter tests, not guest compilation or Chromium evidence.
import test from 'node:test';
import assert from 'node:assert/strict';
import {MonotonicSleepProvider} from './monotonic-sleep.mjs';

function clockHarness(maxPending=2){
  let tick=0,next=0;
  const queue=new Map();
  const sleep=new MonotonicSleepProvider({
    now:()=>tick,
    maxPending,
    schedule:(fn,delay)=>{
      const id=++next;
      queue.set(id,{fn,delay});
      return id;
    },
    unschedule:id=>queue.delete(id)
  });
  return {
    sleep,
    count:()=>queue.size,
    delays:()=>[...queue.values()].map(x=>x.delay),
    fireAt(t){
      assert.ok(queue.size>0,'fake timer must be pending');
      const [id,job]=queue.entries().next().value;
      queue.delete(id);
      tick=t;
      job.fn();
    }
  };
}

test('spurious early wakeups rearm until the true monotonic deadline',async()=>{
  const h=clockHarness();
  let done=false;
  const pending=h.sleep.sleep(100).then(()=>{done=true});
  assert.equal(h.sleep.pending,1);
  assert.deepEqual(h.delays(),[100]);

  h.fireAt(40);
  await Promise.resolve();
  assert.equal(done,false,'early 40ms wake completed the timer');
  assert.deepEqual(h.delays(),[60]);

  h.fireAt(99);
  await Promise.resolve();
  assert.equal(done,false,'early 99ms wake completed the timer');
  assert.deepEqual(h.delays(),[1]);

  h.fireAt(100);
  await pending;
  assert.equal(done,true);
  assert.equal(h.sleep.pending,0);
  assert.equal(h.count(),0);
});

test('zero delay still completes after one scheduler tick',async()=>{
  const h=clockHarness();
  let done=false;
  const pending=h.sleep.sleep(0).then(()=>{done=true});
  await Promise.resolve();
  assert.equal(done,false);
  assert.deepEqual(h.delays(),[0]);
  h.fireAt(0);
  await pending;
  assert.equal(done,true);
  assert.equal(h.sleep.pending,0);
});

test('per-worker quota rejects excess registrations without dropping live timers',async()=>{
  const h=clockHarness(1);
  const pending=h.sleep.sleep(100);
  assert.equal(h.sleep.pending,1);
  assert.throws(()=>h.sleep.sleep(100),/registration budget exhausted/);
  assert.equal(h.sleep.pending,1);
  assert.equal(h.count(),1);
  h.fireAt(100);
  await pending;
  assert.equal(h.sleep.pending,0);
  const second=h.sleep.sleep(1);
  h.fireAt(101);
  await second;
  assert.equal(h.sleep.pending,0);
});

test('abort wins before timer settlement and releases all queue capacity',async()=>{
  const h=clockHarness(1);
  const controller=new AbortController();
  const pending=h.sleep.sleep(1000,controller.signal);
  assert.equal(h.sleep.pending,1);
  assert.equal(h.count(),1);
  controller.abort();
  await assert.rejects(pending,/timer cancelled/);
  assert.equal(h.sleep.pending,0);
  assert.equal(h.count(),0);
  const reused=h.sleep.sleep(0);
  h.fireAt(0);
  await reused;
});

test('invalid duration and invalid clock values fail closed without registering timers',async()=>{
  const h=clockHarness();
  for(const value of [-1,1001,NaN,Infinity,0.5]){
    assert.throws(()=>h.sleep.sleep(value),/invalid or unsupported/);
  }
  assert.equal(h.sleep.pending,0);
  assert.equal(h.count(),0);
  const invalid=new MonotonicSleepProvider({now:()=>NaN});
  assert.throws(()=>invalid.sleep(1),/invalid monotonic clock/);
  assert.equal(invalid.pending,0);
});

test('regressing monotonic readings fail closed and release admission',async()=>{
  const h=clockHarness(1);
  const pending=h.sleep.sleep(100);
  h.fireAt(-1);
  await assert.rejects(pending,/invalid or regressed monotonic clock/);
  assert.equal(h.sleep.pending,0);
  assert.equal(h.count(),0);
  const next=h.sleep.sleep(1);
  h.fireAt(1);
  await next;
  assert.equal(h.sleep.pending,0);
});

test('nonfinite monotonic readings after registration fail closed',async()=>{
  const h=clockHarness();
  const pending=h.sleep.sleep(100);
  h.fireAt(NaN);
  await assert.rejects(pending,/invalid or regressed monotonic clock/);
  assert.equal(h.sleep.pending,0);
  assert.equal(h.count(),0);
});

test('failed scheduling revokes the admission slot',async()=>{
  const invalid=new MonotonicSleepProvider({
    schedule:()=>{throw new Error('scheduler shutdown')}
  });
  await assert.rejects(invalid.sleep(42),/scheduler shutdown/);
  assert.equal(invalid.pending,0);
});
