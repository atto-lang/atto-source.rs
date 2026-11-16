// AttoLang pipe/rpipe failure contract REFERENCE MODEL.
// This is NOT Atto guest execution and does not implement the pipe runtime.
// It is a portable Node 22 behavioral oracle for type/effect invariants.
import test from "node:test";
import assert from "node:assert/strict";

const Ok = value => ({ kind: "Ok", value });
const Err = error => ({ kind: "Err", error });
const Some = value => ({ kind: "Some", value });
const None = { kind: "None" };
const EOF = Symbol("stream-end");
const effect = (kind, message) => Object.assign(new Error(message), { effect: kind });

function trap_once(f) {
  try { return Some(f()); }
  catch (e) {
    if (e?.effect === "throw") return None;
    throw e; // raise, panic, cancellation, host fault bypass
  }
}

function typed_graph(stages, direction) {
  if (stages.length < 2) return Err("InsufficientStages");
  const flow = direction === "rpipe" ? [...stages].reverse() : [...stages];
  for (let i = 0; i + 1 < flow.length; i++) {
    if (!flow[i].output) return Err("MissingSource");
    if (!flow[i + 1].input) return Err("MissingSink");
    if (flow[i].output !== flow[i + 1].input) return Err("TypeMismatch");
  }
  return Ok({ order: flow.map(s => s.name), direction });
}

function read_once(queue, counters) {
  counters.reads++;
  if (queue.closed && queue.items.length === 0) return Ok(None);
  if (queue.error) return Err(queue.error);
  if (queue.items.length === 0) return Err("Pending");
  return Ok(Some(queue.items.shift()));
}
function write_once(sink, value, counters) {
  counters.writes++;
  if (sink.closed) return Err("Closed");
  sink.items.push(value);
  return Ok({ admitted: 1 });
}
function apply_forward(value, fn) { return fn(value); }
function apply_reverse(fn, value) { return fn(value); }

test("single >> and << are one operation, not an implicit pump", () => {
  const q={ items: [11, 12], closed: true };
  const sink={ items: [] };
  const c={ reads: 0, writes: 0 };
  assert.deepEqual(read_once(q,c),Ok(Some(11)));
  assert.deepEqual(write_once(sink,22,c),Ok({admitted:1}));
  assert.deepEqual(q.items,[12]);
  assert.deepEqual(sink.items,[22]);
  assert.deepEqual(c,{ reads: 1,writes:1 });
  assert.deepEqual(read_once({items:[],closed:true},c),Ok(None));
  assert.notDeepEqual(Ok(None),Err("IoError"));
});

test("pipe and rpipe require opposite lexical directions but same flow", () => {
  const source={name:"source",input:null,output:"int"};
  const transform={name:"transform",input:"int",output:"bool"};
  const sink={name:"sink",input:"bool",output:null};
  const f=typed_graph([source,transform,sink],"pipe");
  const r=typed_graph([sink,transform,source],"rpipe");
  assert.equal(f.kind,"Ok");
  assert.equal(r.kind,"Ok");
  assert.deepEqual(f.value.order,r.value.order);
  assert.deepEqual(r.value.order,["source","transform","sink"]);
  assert.equal(typed_graph([source,sink],"rpipe").error,"MissingSource");
  assert.equal(typed_graph([source,{name:"wrong",input:"str"}],"pipe").error,"TypeMismatch");
});

test("functional |> / <| evaluate only one value and retain checked Result", () => {
  let calls=0;
  const fn=x=>{calls++;return Ok(x+1);};
  assert.deepEqual(apply_forward(41,fn),Ok(42));
  assert.deepEqual(apply_reverse(fn,41),Ok(42));
  assert.equal(calls,2); // one for each independent application
  assert.deepEqual(apply_reverse(x=>Err("bad"),1),Err("bad"));
});

test("trap wraps ordinary throw, not returned Result.Err", () => {
  let count=0;
  assert.deepEqual(trap_once(()=>{count++;return Err("IO");}),Some(Err("IO")));
  assert.deepEqual(trap_once(()=>{count++;throw effect("throw","ordinary");}),None);
  assert.equal(count,2);
  assert.notDeepEqual(Some(Err("IO")),None);
});

test("trap does not swallow raise, panic, cancel or host error", () => {
  for (const kind of ["raise","panic","cancel","host"]) {
    assert.throws(()=>trap_once(()=>{throw effect(kind,kind);}),e=>e.effect===kind);
  }
});

test("Option.None data is not stream EOF; Result.Err data is not pipe failure", () => {
  const packets=[Some(4),None,Err("payload-level")];
  assert.equal(packets.length,3);
  assert.equal(packets[1].kind,"None");
  assert.notEqual(packets[1],EOF);
  assert.equal(packets[2].kind,"Err");
  assert.notEqual(packets[2],Err("PipeError"));
});

test("checked pipe completion cannot be retroactively swallowed by setup trap", () => {
  const handle={ get_done:()=>Err({kind:"PipeError",reason:"Cancelled",committed:2}) };
  const maybe_handle=trap_once(()=>handle);
  assert.equal(maybe_handle.kind,"Some");
  assert.deepEqual(trap_once(()=>maybe_handle.value.get_done()),Some(Err({
    kind:"PipeError",reason:"Cancelled",committed:2
  })));
});

test("graph failure is separate from actor CONTROL and preserves committed count", () => {
  const complete={kind:"Err",error:{kind:"StageFailure",committed:2}};
  const control={kind:"panic",actor_id:7,generation:1};
  assert.equal(complete.error.committed,2);
  assert.notEqual(complete.error.kind,control.kind);
  assert.equal(trap_once(()=>complete).kind,"Some");
});
