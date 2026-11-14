# Compiled guest mailbox trace refinement — browser Wasm reference

Status: **compiler artifact and guest execution required; workflow result is the evidence, not this document.**

The preexisting `tests/conformance.mjs` compiles `.atto` fixtures using the actual
browser-Wasm compiler, instantiates the resulting Wasm guests and calls
`actor_receive`. This PR adds an optional event sink at those guest/host
boundaries and validates every positive fixture's mailbox history with the
independent `tests/actor-trace-refinement.mjs` oracle.

### Events

```text
actor_spawn(actor_id, generation)
send_accept(actor_id, generation, message_id)
receive_start(actor_id, generation, message_id)
receive_complete(actor_id, generation, message_id)
actor_close(actor_id, generation)
```

IDs are allocated monotonically in a single `runGuest` instance. The
browser reference guest host has generation 1 and executes its actor mailboxes
serially after main returns. This is a **legitimate scheduling subset**, not
the concurrent actor contract. Each accepted message must start receive in
per-actor FIFO order, complete exactly once, and be fully accounted for when
its actor is closed. For these successful fixtures no accepted delivery can
silently disappear or be double-settled.

The oracle rejects unknown actor IDs, stale generations, forged actions,
duplicate admission, wrong FIFO identity, phantom dispatch, wrong/double
settlement, close-before-drain, post-close sends, missing terminal events,
unadmitted messages and malformed IDs. At least one actor and two successful
deliveries must actually execute, preventing vacuous green fixtures.

### Commands

```sh
node --test tests/actor-trace-refinement.test.mjs
# Compiles the same .atto fixture corpus via the browser-wasm compiler and
# executes all positive guests through the instrumented browser host:
node tests/conformance.mjs target/wasm32-unknown-unknown/release/atto_compiler.wasm
```

The existing CI workflow builds the compiler with pinned Rust 1.85.1, runs
the Node negative tests, and executes the actual compiled Wasm guests. Its
exact-head checkout, compiler digest and all job step results are required
before claiming any PASS. A source file alone is NOT a guest execution receipt.

### Non-claims

- These events are observed at the browser host's Wasm import/export boundary,
  **not by instrumenting machine-code stack frames or every instruction**.
- They prove only that the observed, successful finite test histories conform
  to the oracle, not all possible histories or an unbounded theorem.
- They do not cover an actor restart, physical heap reclamation, crash recovery,
  OS isolation, simultaneous receives, critical read/write scopes, Java,
  LLVM, .NET NativeAOT, Forge or other independent backends.
- There is no successful compiled guest proof until an exact-head runner
  executes the compiler build, complete conformance test and negative suite.

Next: implement a shared versioned JSONL trace contract with actor generations,
linearization points, CONTROL priority, cancellation and terminal barriers,
then add independent adapters for production runtimes; test true interleavings
using a model checker instead of treating this serial-host oracle as full parity.