# AttoLang `Mutex<int>` closed-world guest fast path (experimental)

**Status: implementation candidate, NOT_EXECUTED on the current source head.** This is a native Rust/browser-Wasm compiler slice for source-compiled Atto guest code, not a released general mutex API or an actor/TenantVM scheduler.

## Admitted source

Only exactly two functions are supported: one **private** `susp`/`suspends fnc increment(Mutex<int> handle): int` with a lexical `using [await] handle.lock_async() as lease` body that performs a checked `lease.value += N` and returns a copied scalar, and one public `main` constructing a fresh `new Mutex<int>(value)` nominal resource class from a signed integer literal. The main function may await the increment once, print the result and return 0, or await it twice **sequentially** (second acquisition proves release of the first). A non-suspending `main` may create and drop an unawaited lazy task without running it.

Every other shape, including new actors, extra functions, mutable aliases, external callbacks, generic payloads, nonconstant arithmetic, exceptional unwind, tasks driven concurrently, `spawn`, `select`, `yield`, other resources or cross-domain handles, MUST fail compilation without guest bytes. This profile is not a claim that `Mutex<T>` is implemented in the ordinary parser/stdlib.

## Real lowering — not a constant-result stub

The bounded parser resolves owner/lease identifiers, effects, source tokens and signed arithmetic. `mutex_lease_hir_v1.rs` adds an explicit, noncopyable affine `MutexLease<int>` type-state model: only the active state has a consuming `release` method, and the lowerer constructs the return path after its static release operation. The HIR proof checks every sequential guarded addition for overflow before the guest can execute. The emitted typed `TProgram` uses the ordinary checked i64 add helper and two module-private Wasm globals: protected scalar payload and lexical lease-occupancy flag. `increment` checks the flag, sets it, executes the addition, snapshots the result, clears the flag, and only then returns. A second sequential await would trap at acquisition if the first left the flag busy. Guest `main` initializes the payload and calls the private helper; it is the only export that can drive it. The unawaited case emits no call to the helper.

Because the compiler proves there is only one task, no reentrant host import or actor, no dynamic exception inside the guard (constant checked overflow fails at compile time), and no eligible competing mutex holder, **acquisition is unconditionally ready**, so no host scheduler import is needed. The source `await` effect and typed lease cannot be generalized from this optimization: pending waits require a separately versioned async ABI, task continuation and bounded wakeup runtime.

The host-model `mutex_wait_queue_v1.rs` is compiled only under `#[cfg(test)]`; its previous funded exact-blob test receipt is **not a guest result**.

## Acceptance tests and security boundary

`cargo test --locked --test mutex_fast_guest_profile` and `cargo test --locked --lib` test compiler source contracts. After building the actual native `attoc` host compiler, run `ATTOC=target/release/attoc ATTO_SOURCE_SHA=<exact-40-hex-head> node tests/mutex_fast_guest_execution.mjs` to compile the thirteen `.atto` fixtures, execute positive Wasm guests, test byte-identical implicit/explicit spellings, different scalar results and sequential cleanup, and require exact negative errors without artifacts. The CI workflow includes this step, but source-org and alternate-carrier Actions have not admitted runnable steps on this current head.

Never mark `atto-platform-conformance` mutex backend PASS from this profile. General mutex admission still needs arbitrary-payload typed HIR/MIR leases (the current HIR model is **only** the ready scalar `MutexLease<int>` specialization), cancellation-safe `using` unwind, a genuinely nonparking contended waiter, a new negotiated async host/guest ABI and adversarial actor/TenantVM verification. The legacy JSPI `atto.async/2` mandatory-yield PRs are a separate, unreconciled contract.

## Source-grammar negative diagnostics (new)

Bare resource constructors are invalid independently of lock use: `Mutex<int>(0)` and `Mutex<Result<int,string>>(0)` are rejected with `E_NEW_REQUIRED` even in a file without `using` or `.lock_async()`. The scanner uses bounded nested type-argument token tracking, ignores quoted/commented text, and distinguishes `Mutex<int>` type references from constructor calls. `new Mutex<T>` with unsupported payload or unverified synchronization remains fail-closed; recognizing the `new` token does not grant a runtime capability.

**Verification status as of this branch update:** the implementation, thirteen guest tests and four affine lease HIR unit tests are *authored but NOT EXECUTED*. The last source-org Actions jobs and the approved private carrier's admission probe ([athlet-o run #44681245731](https://github.com/athlet-o/atto-ci/actions/runs/44681245731)) failed before runner steps, so they cannot be treated as compiler or guest failures/passes. The historic carrier PASS applied only to an older, test-only host waiter model. No PR should merge or assert backend parity without executing the real exact-head crate and Wasm guest tests.
