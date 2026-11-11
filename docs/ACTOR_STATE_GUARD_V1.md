# Guarded shared-actor state: FIFO reader/writer admission V1

**Status:** a bounded Rust *logical admission model*. This does **not** lower
AttoLang \`state {}\` or \`handle_state\`, allocate shared memory, provide
OS/VM isolation, or prove actual runtime synchronization. This module is a
separate follow-up to the browser compiler's single-Worker lexical scopes.

\`actor_state_guard_v1::GuardQueue\` is owned by the trusted runtime, **not**
by any actor/supervisor. It issues opaque non-owning permits only for an actor
identity authenticated by the runtime. The integer identities and permits do not
constitute cross-tenant authority. Do not expose these Rust methods or permits
to guest code. A runtime integration must bind actor identity + generation +
region capability cryptographically or through unforgeable handles.

## Ordering and fairness

- Requests are enqueued FIFO. Admit the oldest writer exclusively, or a
  contiguous reader prefix, including concurrent readers.
- Once a writer is waiting, later readers cannot pass it (bounded writer
  starvation subject to holders eventually releasing).
- Admission does not block a carrier; callers schedule notification to actors
  out of band after \`admit_ready\`.
- The queue rejects reentrant state guards for the same actor and caps
  outstanding queued + admitted permits at 1024 per region.
- \`access\` validates holder, generation, active permit and exclusive write.
  A permit never returns a memory address or a Rust reference to actor state.
- \`cancel_pending\` removes only unadmitted requests; cancellation of active
  code requires an actual guest quiescence protocol.

## Retirement and physical safety

\`retire(new_generation)\` fences all *logical* accesses and drops outstanding
waiters. It deliberately retains active permits, even after a STOP or hot
upgrade. Ordinary \`release\` can no longer reclaim those leases.

\`complete_physical_quiescence\` is a **trusted runtime hook** that must be
called only after independent proof that the actor/callback/carrier has stopped
touching the region. A guest acknowledgement or generation ID alone is not
proof. The model reports \`may_reclaim\` only when retired and all active
permits have completed quiescence. Allocation destruction and byte wiping
require an additional runtime-domain proof.

## Not yet certified

- Actual reader/writer lock linearization under multi-threaded races.
- Supervisor STOP/cancellation/rollback callbacks and physical quiescence.
- Cross-actor/tenant handle authentication, quotas or memory allocation.
- Compiler-generated \`critical-read\`/\`critical-write\` effect/census parity.
- Native, Wasm, JVM and NativeAOT guest execution.

Required follow-up: wire a trusted region manager into actor scheduling, use
generation-fenced non-escapable guards, run actual concurrent positive and
adversarial races with cancellation, and demonstrate same semantic negatives
on Java, LLVM, .NET, Forge and Wasm compiled guests. Model tests are not guest
proof, even if executed by CI.
