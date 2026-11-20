# AttoLang `Mutex<int>` closed-world guest fast path (experimental)

**Status: implementation candidate, NOT_EXECUTED on the current source head.** This is a native Rust/browser-Wasm compiler slice for source-compiled Atto guest code, not a released general mutex API or an actor/TenantVM scheduler.

## Admitted source

Only exactly two functions are supported: one **private** `susp`/`suspends fnc increment(Mutex<int> handle): int` with a lexical `using [await] handle.lock_async() as lease` body that performs a checked `lease.value += N` and returns a copied scalar, and one public `main` constructing a fresh `Mutex<int>` from a signed integer literal. The main function may await the increment once, print the result and return 0, or await it twice **sequentially** (second acquisition proves release of the first). A non-suspending `main` may create and drop an unawaited lazy task without running it.

Every other shape, including new actors, extra functions, mutable aliases, external callbacks, generic payloads, nonconstant arithmetic, exceptional unwind, tasks driven concurrently, `spawn`, `select`, `yield`, other resources or cross-domain handles, MUST fail compilation without guest bytes. This profile is not a claim that `Mutex<T>` is implemented in the ordinary parser/stdlib.

## Real lowering — not a constant-result stub

The bounded parser resolves owner/lease identifiers, effects, source tokens and signed arithmetic. The emitted typed `TProgram` uses the ordinary checked i64 add helper and two module-private Wasm globals: protected scalar payload and lexical lease-occupancy flag. `increment` checks the flag, sets it, executes the addition, snapshots the result, clears the flag, and only then returns. A second sequential await would trap at acquisition if the first left the flag busy. Guest `main` initializes the payload and calls the private helper; it is the only export that can drive it. The unawaited case emits no call to the helper.

Because the compiler proves there is only one task, no reentrant host import or actor, no dynamic exception inside the guard (constant checked overflow fails at compile time), and no eligible competing mutex holder, **acquisition is unconditionally ready**, so no host scheduler import is needed. The source `await` effect and typed lease cannot be generalized from this optimization: pending waits require a separately versioned async ABI, task continuation and bounded wakeup runtime.

The host-model `mutex_wait_queue_v1.rs` is compiled only under `#[cfg(test)]`; its previous funded exact-blob test receipt is **not a guest result**.

## Acceptance tests and security boundary

`cargo test --locked --test mutex_fast_guest_profile` and `cargo test --locked --lib` test compiler source contracts. After building the actual native `attoc` host compiler, run `ATTOC=target/release/attoc ATTO_SOURCE_SHA=<exact-40-hex-head> node tests/mutex_fast_guest_execution.mjs` to compile the ten `.atto` fixtures, execute positive Wasm guests, test byte-identical implicit/explicit spellings, different scalar results and sequential cleanup, and require exact negative errors without artifacts. The CI workflow includes this step, but source-org and alternate-carrier Actions have not admitted runnable steps on this current head.

Never mark `atto-platform-conformance` mutex backend PASS from this profile. General mutex admission still needs typed HIR/MIR leases, cancellation-safe `using` unwind, a genuinely nonparking contended waiter, a new negotiated async host/guest ABI and adversarial actor/TenantVM verification. The legacy JSPI `atto.async/2` mandatory-yield PRs are a separate, unreconciled contract.
