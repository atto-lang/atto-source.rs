# Owned pending Future AOT unwind candidate

Current-main forward port of the externally-pending continuation approach in #197,
with a distinct failure-aware ABI. The older v5 arena and its trusted host
remain separate; this does not supersede the multi-frame/ActorFuture work.
Opt-in `attoc --owned-unwind-v1 input.atto -o guest.wasm` uses production
tokens, typed affine formal slots and compiler-generated Wasm state transitions.
The default compiler still rejects unsupported sealed Futures.

Exactly the two shared owned pending fixtures are supported, plus variations
within the closed grammar: up to eight unique take! Future<int> formals,
braceless defer await, integer or awaited return, at most one recovery
callback with an awaited return. Each formal has exactly one lexical use.
No dynamic call, arbitrary expression, capture, loop, generic Future,
actor spawn, or guest construction.

ABI: all calls start with positive owner and epoch i32. One invocation per
instance; no restart. start returns 1 pending; pending returns the 1-based
formal slot. complete(owner,epoch,slot,status,value) accepts status 0 value,
1 failure, 2 cancellation; failure/cancel value is positive opaque i64
error receipt. It only records readiness; resume returns 1 pending or
2 terminal. Invalid nonquery operations return -1; invalid terminal queries
trap. outcome is 1 return, 2 recovered, 3 raise, 4 panic, 5 cancel.
result is readable only on success; error and suppressed error/rank are
terminal receipts; rank 1 raise, 2 panic, 3 cancel. retire is terminal-only,
clears payloads and permanently seals instance. No imports, memory or table
exports, no guest GC, no interpreter. Trusted host still must bind the
numeric owner/epoch and error receipt to real identity and one-shot
completion authority, enforce exclusive Future handoff and scheduler lanes.
This profile does NOT implement the authenticated host adapter, actor
STOP deadlines, full physical actor retirement, general typed MIR or full
cross-backend parity. Node test drives real Wasm exports, not host simulation.
