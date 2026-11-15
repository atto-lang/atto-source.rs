# Critical state admission: ten-scenario compiler audit

Applies to the restricted Atto Rust-to-Wasm Worker-local **mutable state container**
profile. The results of the matrix must be recorded only after running its
Rust and native/Wasm guest conformance checks on the exact change SHA.

| # | Fixture | Expect |
| - | - | - |
| 1 | pass/210-critical-alias-int.atto | Exclusive `critical` mutation, stdout 8 |
| 2 | pass/211-critical-write-bool.atto | Explicit write, stdout true then false |
| 3 | pass/212-critical-read-snapshot.atto | By-value snapshot, stdout 4 then 9 |
| 4 | pass/213-critical-two-containers.atto | Separate owners, stdout 8, 13, 8 |
| 5 | fail/210-critical-read-block-escalation.atto | E_STATE_BORROW, no artifact |
| 6 | fail/211-critical-read-mutation.atto | E_STATE_BORROW, no artifact |
| 7 | fail/212-critical-unguarded-self.atto | E_STATE_ACCESS, no artifact |
| 8 | fail/213-critical-binder-escape.atto | E_ESCAPE, no artifact |
| 9 | fail/214-critical-spaced-keyword.atto | E_SYNTAX, no artifact |
| 10 | fail/215-critical-nested-loan.atto | E_STATE_BORROW, no artifact |

## Hardening rule

`critical` and `critical-write` are exclusive-write modes in this subset.
`critical-read` is an immutable *by-value scalar field projection*:
`handle_state(critical-read |s| -> s.field)`. A `critical-read` block,
even an empty one, is an error. Previously that shape could be typed as
`StateCriticalWrite`, incorrectly minting an exclusive write ticket.

Only owned scalar `int` and `bool` fields are admitted for this profile.
The mutable **container** is the protected, owned state object; this does
not yet support mutable `Array`, `Map`, `Set` or arbitrary reference
graphs inside protected state. Reads must not expose pointers or references.

## Evidence boundaries

`cargo test --test critical_state_matrix` and `cargo test --test fixtures`
check native compiler acceptance, diagnostics and determinism. The existing
`tests/conformance.mjs` executes the new pass fixtures as real generated
Wasm guests for stdout / exit assertions and fail fixtures for no-artifact
rejection. A real browser Worker run is an additional integration gate.

The current implementation is **single Worker-local**: its host-mediated
synchronous tickets are not a cross-Worker RW lock and do not provide
suspending reads, asynchronous lock upgrades, actor migration, or general
lifecycle/cancellation/reclamation guarantees. This document is an audit
contract, not evidence that any unexecuted CI test passed.
