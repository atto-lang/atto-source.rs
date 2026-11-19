// Observable mailbox-operation trace refinement for the browser Wasm guest ABI.
// This checks host-observed import/export boundary events AFTER the actual .atto
// source has been compiled into a Wasm guest module and executed.
// It is not an OS-thread, generation-restart, LLVM, or Java proof.
import assert from 'node:assert/strict';

const EVENTS = new Set([
  'actor_spawn', 'send_accept', 'receive_start', 'receive_complete', 'actor_close',
]);
// These bounds are tied to the intentionally SERIAL browser reference host,
// not to the AttoLang actor model or production runtimes.
export const BROWSER_MAX_ACTORS = 16;
export const BROWSER_MAX_MAILBOX = 10_000;
// Each message has one admit, one receive start, and one completion event.
const MAX_EVENTS = BROWSER_MAX_ACTORS * (3 * BROWSER_MAX_MAILBOX + 2);

function natural(value, label, allowZero = false) {
  assert.equal(typeof value, 'number', label + ' must be an integer');
  assert.ok(Number.isSafeInteger(value) && (allowZero ? value >= 0 : value > 0),
    label + ' must be a nonnegative/safe integer');
}

export function verify_actor_trace(events) {
  assert.ok(Array.isArray(events), 'trace must be an array');
  assert.ok(events.length <= MAX_EVENTS, 'trace event budget exceeded');

  const actors = new Map();
  let accepted = 0, completed = 0;
  let draining = false, drainingActor = 0;
  for (const [index, event] of events.entries()) {
    assert.ok(event && typeof event === 'object' && !Array.isArray(event),
      'event ' + index + ' is not an object');
    assert.ok(EVENTS.has(event.kind), 'unknown trace action at ' + index);
    const hasMessage = event.kind !== 'actor_spawn' && event.kind !== 'actor_close';
    assert.deepEqual(Object.keys(event).sort(),
      (hasMessage ? ['actor_id', 'generation', 'kind', 'message_id']
                  : ['actor_id', 'generation', 'kind']).sort(),
      'unexpected or missing trace fields at ' + index);
    natural(event.actor_id, 'actor_id', true);
    natural(event.generation, 'generation');
    if (event.kind === 'actor_spawn') {
      assert.ok(!draining, 'spawn after browser mailbox draining began');
      assert.ok(actors.size < BROWSER_MAX_ACTORS, 'actor admission exceeds browser limit');
      // The browser host monotonically allocates actor slots and never recycles.
      assert.equal(event.actor_id, actors.size, 'non-monotonic actor id allocation');
      assert.equal(event.generation, 1, 'browser host actor generation must be one');
      assert.ok(!actors.has(event.actor_id), 'actor slot reused');
      actors.set(event.actor_id, {
        generation: event.generation, phase: 'live', ready: [],
        in_flight: null, next_message_id: 1,
        accepted_ids: new Set(), completed_ids: new Set(),
      });
      continue;
    }
    const actor = actors.get(event.actor_id);
    assert.ok(actor, 'unknown or not-yet-spawned actor');
    assert.equal(event.generation, actor.generation, 'stale actor generation');
    assert.equal(actor.phase, 'live', 'operation after terminal close');
    if (event.kind === 'actor_close') {
      draining = true;
      assert.equal(event.actor_id, drainingActor, 'actor drain order changed');
      assert.equal(actor.ready.length, 0, 'close with queued messages');
      assert.equal(actor.in_flight, null, 'close with in-flight message');
      actor.phase = 'closed';
      drainingActor++;
      continue;
    }
    natural(event.message_id, 'message_id');
    if (event.kind === 'send_accept') {
      assert.ok(!draining, 'send after mailbox draining began');
      assert.ok(actor.ready.length < BROWSER_MAX_MAILBOX, 'mailbox capacity exceeded');
      assert.equal(event.message_id, actor.next_message_id,
        'message IDs must be consecutive in this browser host');
      actor.next_message_id++;
      assert.ok(!actor.accepted_ids.has(event.message_id), 'duplicate admission');
      actor.accepted_ids.add(event.message_id);
      actor.ready.push(event.message_id);
      accepted++;
    } else if (event.kind === 'receive_start') {
      draining = true;
      assert.equal(event.actor_id, drainingActor, 'out-of-order actor draining');
      assert.equal(actor.in_flight, null, 'two simultaneous receives in serial Wasm host');
      assert.ok(actor.ready.length > 0, 'receive without accepted message');
      assert.equal(actor.ready[0], event.message_id, 'out-of-order FIFO receive');
      actor.ready.shift();
      actor.in_flight = event.message_id;
    } else if (event.kind === 'receive_complete') {
      draining = true;
      assert.equal(event.actor_id, drainingActor, 'out-of-order actor completion');
      assert.equal(actor.in_flight, event.message_id,
        'completion without matching in-flight delivery');
      assert.ok(!actor.completed_ids.has(event.message_id), 'duplicate settlement');
      actor.completed_ids.add(event.message_id);
      actor.in_flight = null;
      completed++;
    }
  }
  for (const [actor_id, actor] of actors) {
    assert.equal(actor.phase, 'closed', 'actor missing terminal close: ' + actor_id);
    assert.equal(actor.ready.length, 0, 'unaccounted accepted messages');
    assert.equal(actor.in_flight, null, 'unsettled actor delivery');
    assert.equal(actor.accepted_ids.size, actor.completed_ids.size,
      'message conservation');
  }
  assert.equal(accepted, completed, 'accepted work was lost');
  return { actor_count: actors.size, admitted: accepted, completed };
}
