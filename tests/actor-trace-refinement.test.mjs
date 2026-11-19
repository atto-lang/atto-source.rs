import test from 'node:test';
import assert from 'node:assert/strict';
import { verify_actor_trace } from './actor-trace-refinement.mjs';

const event = (kind, actor_id, message_id, generation = 1) => ({
  kind, actor_id, generation, ...(message_id === undefined ? {} : { message_id }),
});
const valid = [
  event('actor_spawn', 0),
  event('actor_spawn', 1),
  event('send_accept', 0, 1),
  event('send_accept', 0, 2),
  event('send_accept', 1, 1),
  event('receive_start', 0, 1),
  event('receive_complete', 0, 1),
  event('receive_start', 0, 2),
  event('receive_complete', 0, 2),
  event('actor_close', 0),
  event('receive_start', 1, 1),
  event('receive_complete', 1, 1),
  event('actor_close', 1),
];
const copy = () => structuredClone(valid);

test('accepts complete actual-host-shaped FIFO traces', () => {
  assert.deepEqual(verify_actor_trace(valid), { actor_count: 2, admitted: 3, completed: 3 });
  assert.deepEqual(verify_actor_trace([]), { actor_count: 0, admitted: 0, completed: 0 });
});

const mutations = [
  ['out-of-order FIFO', events => { events[5].message_id = 2; }],
  ['stale actor generation', events => { events[2].generation = 2; }],
  ['duplicate admission', events => { events.splice(3, 0, event('send_accept', 0, 1)); }],
  ['phantom receive', events => { events[5].message_id = 19; }],
  ['wrong completion identity', events => { events[6].message_id = 2; }],
  ['duplicate settlement', events => { events.splice(7, 0, event('receive_complete', 0, 1)); }],
  ['premature close', events => { events.splice(5, 0, event('actor_close', 0)); }],
  ['post-termination send', events => { events.push(event('send_accept', 0, 4)); }],
  ['unknown actor', events => { events[2].actor_id = 9; }],
  ['missing completion', events => { events.splice(6, 1); }],
  ['missing terminal action', events => { events.splice(9, 1); }],
  ['unadmitted message', events => { events.splice(2, 1); }],
  ['invalid message identity', events => { events[2].message_id = -1; }],
  ['invalid event', events => { events[2].kind = 'forged_host_receipt'; }],
  ['message ID gap', events => { events[2].message_id = 4; }],
  ['message ID non-consecutive for second send', events => { events[3].message_id = 7; }],
  ['unknown trace field', events => { events[2].payload = 'forged'; }],
  ['missing required trace field', events => { delete events[5].generation; }],
  ['close carrying message ID', events => { events[9].message_id = 1; }],
  ['send during drain', events => { events.splice(6, 0, event('send_accept', 1, 2)); }],
  ['spawn after drain', events => { events.push(event('actor_spawn', 2)); }],
  ['wrong actor drain order', events => { events[5].actor_id = 1; }],
  ['wrong actor close order', events => { events[9].actor_id = 1; }],
  ['non-integer actor ID', events => { events[2].actor_id = 0.5; }],
  ['non-safe message ID', events => { events[2].message_id = Number.MAX_SAFE_INTEGER + 1; }],
];
for (const [name, modify] of mutations) {
  test('refinement rejects mutation: ' + name, () => {
    const events = copy();
    modify(events);
    assert.throws(() => verify_actor_trace(events), /./, 'mutation must be rejected: ' + name);
  });
}

test('browser-specific limits reject 17 actors', () => {
  const events = Array.from({ length: 17 }, (_, i) => event('actor_spawn', i));
  assert.throws(() => verify_actor_trace(events), /actor admission exceeds browser limit/);
});
test('browser-specific limits reject 10001 outstanding sends', () => {
  const events = [event('actor_spawn', 0)];
  for (let id = 1; id <= 10_001; id++) events.push(event('send_accept', 0, id));
  assert.throws(() => verify_actor_trace(events), /mailbox capacity exceeded/);
});
