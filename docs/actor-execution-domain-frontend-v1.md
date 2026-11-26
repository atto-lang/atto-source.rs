# Actor execution class frontend slice v1

**Branch status: DRAFT / no executed CI/guest receipt.** Specification: [atto-spec #49](https://github.com/atto-lang/atto-spec/pull/49). Target conformance: [atto-platform-conformance #43](https://github.com/atto-lang/atto-platform-conformance/pull/43). This extends the existing [typed receives/sends PR #406](https://github.com/atto-lang/atto-source.rs/pull/406), not a separate parser fork.

## What this frontend change implements

- AST `ActorExecutionDomain` is a distinct `Shared | Isolated | Untrusted` value, not inferred from method names, traits, ActorRef types, inheritance, `ActorMail<T>`, or persistence hooks.
- `define actor` remains shared by default; `define shared actor` records shared explicitly and currently uses the same *trusted* actor lowering.
- `define isolated actor` and `define untrusted actor` are parsed and preserved, then **rejected at the compiler checker** with `E_ACTOR_DOMAIN_RUNTIME_UNAVAILABLE` or `E_UNTRUSTED_SANDBOX_UNAVAILABLE`. There is no private worker-pool or security sandbox backend in this frontend. Do not treat AST acceptance as runtime support.
- `define isolated durable actor` remains syntactically distinct from storage policy, but durability itself is still nonexecutable in the browser guest. Out-of-order, repeated, and conflicting modifiers reject at parser admission.
- `define pub ... actor` stays explicit `E_UNSUPPORTED` until a public-actor export/linker contract exists. Never erase a visibility marker. `serial http actor` forms and synthesized domain traits are not implemented by this restricted parser.
- The source-host security floor is NOT a source annotation. An untrusted principal declaring an ordinary actor cannot gain host shared-pool admission. That runtime admission boundary is not exercised by these tests.

## New checked-in .atto fixtures

- `pass/actor-explicit-shared-domain-v1.atto`: shared syntactic alias, expected guest stdout 42.
- `fail/actor-isolated-no-private-runtime-v1.atto`: no silent private→shared fallback.
- `fail/actor-untrusted-no-sandbox-v1.atto`: deny hostile runtime without independently verified TenantVM.
- `fail/actor-domain-conflict-v1.atto`, `actor-domain-duplicate-v1.atto`, and `actor-domain-storage-order-v1.atto`: parser rejects inconsistent modifier sets.

The repository's existing Rust `tests/fixtures.rs` harness iterates all `.atto` files in the pass and fail corpora; a passing Rust test still needs independent guest execution and a stepful exact-head receipt before security claims.

## Next backend gates

1. Authenticate host tenant admission and attach a runtime-enforced principal floor to every actor/continuation.
2. Make private scheduling and mailbox admission/teardown atomic; no fallback to shared workers.
3. Deliver isolated actors only after private carrier capability and bound reentrancy/teardown are tested.
4. Deliver untrusted actors only after actual validated guest confinement, restricted imports, host socket broker and adversarial same-PID evidence (or explicitly labeled sandboxed OS-process fallback).
5. Pin compiler SHA, artifact digest, full exact-tree CI, positive guest 42 and all expected negative diagnostics. Tests marked unexecuted until then.
