import assert from 'node:assert/strict';
import test from 'node:test';
import {createStateRuntime} from './browser/state-runtime.mjs';

test('host-issued i64 ticket, closed without replay, retirement fenced', () => {
  const events=[], runtime=createStateRuntime({onEvent:e=>events.push(e.kind)});
  const {state_register, state_enter, state_leave}=runtime.imports;
  state_register(0);
  state_register(8);
  const a=state_enter(0,0);
  const b=state_enter(0,0);
  assert.equal(typeof a,'bigint');
  assert.notEqual(a,b);
  assert.equal(runtime.snapshot().active,2);
  assert.throws(()=>state_enter(0,1),/contention/);
  assert.throws(()=>state_enter(8,9),/invalid lease mode/);
  assert.throws(()=>state_enter(16,0),/unknown state container/);
  runtime.beginRetire(0);
  assert.equal(runtime.quiescent(0),false);
  assert.throws(()=>state_enter(0,0),/retired/);
  state_leave(a);
  state_leave(b);
  assert.equal(runtime.quiescent(0),true);
  assert.throws(()=>state_leave(a),/replayed admission ticket/);
  const c=state_enter(8,1);
  state_leave(c);
  runtime.complete();
  assert.deepEqual(runtime.snapshot().active,0);
  assert.deepEqual(events.slice(0,2),['register','register']);
  assert.throws(()=>state_register(24),/after retirement/);
});

test('different guest instances cannot replay another incarnation ticket', () => {
  const left=createStateRuntime(), right=createStateRuntime();
  left.imports.state_register(0);
  right.imports.state_register(0);
  const t=left.imports.state_enter(0,1);
  assert.throws(()=>right.imports.state_leave(t),/replayed admission ticket/);
  assert.notEqual(right.imports.state_enter(0,1),t);
  left.imports.state_leave(t);
  left.complete(); right.abort();
});

test('physical trap unwinding abort clears authority and prevents resume', () => {
  const runtime=createStateRuntime();
  runtime.imports.state_register(0);
  const t=runtime.imports.state_enter(0,1);
  assert.throws(()=>runtime.complete(),/unreleased state tickets/);
  assert.equal(runtime.snapshot().active,1);
  runtime.abort();
  assert.deepEqual(runtime.snapshot().active,0);
  assert.equal(runtime.snapshot().quiescent,true);
  assert.throws(()=>runtime.imports.state_leave(t),/replayed admission ticket/);
  assert.throws(()=>runtime.imports.state_enter(0,0),/retired/);
});

test('bounded instances and forged pointer fail closed', () => {
  const runtime=createStateRuntime({maxContainers:1,maxTickets:1});
  assert.throws(()=>runtime.imports.state_register(-1),/invalid state pointer/);
  assert.throws(()=>runtime.imports.state_register(2),/invalid state pointer/);
  runtime.imports.state_register(0);
  assert.throws(()=>runtime.imports.state_register(0),/pointer reused/);
  assert.throws(()=>runtime.imports.state_register(8),/container admission capacity/);
  const t=runtime.imports.state_enter(0,0);
  assert.throws(()=>runtime.imports.state_enter(0,0),/admission capacity/);
  runtime.imports.state_leave(t);
  runtime.complete();
});

test('distinct mutable containers hold simultaneous independent write leases', () => {
  const runtime = createStateRuntime();
  const host = runtime.imports;
  host.state_register(0);
  host.state_register(8);
  const left = host.state_enter(0, 1);
  const right = host.state_enter(8, 1);
  assert.equal(runtime.snapshot().active, 2);
  assert.throws(() => host.state_enter(0, 0), /contention/);
  assert.throws(() => host.state_enter(8, 1), /contention/);
  host.state_leave(right);
  host.state_leave(left);
  runtime.complete();
  assert.equal(runtime.snapshot().quiescent, true);
});

test('read-only tickets do not upgrade and cannot replay into a write lease', () => {
  const runtime = createStateRuntime();
  const host = runtime.imports;
  host.state_register(0);
  const readA = host.state_enter(0, 0);
  const readB = host.state_enter(0, 0);
  assert.throws(() => host.state_enter(0, 1), /contention/);
  assert.throws(() => host.state_leave(readA + 5000n), /missing or replayed/);
  host.state_leave(readA);
  host.state_leave(readB);
  const write = host.state_enter(0, 1);
  assert.notEqual(write, readA);
  assert.throws(() => host.state_leave(readA), /replayed/);
  host.state_leave(write);
  runtime.complete();
});
