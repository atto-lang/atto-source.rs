# alloc foundations guest cases (NOT YET EXECUTED)

Run each against the compiler-owned `atto.alloc/*` resolver with a real guest
compiler and runtime; record exact source HEAD, toolchain, job steps and result.

| Fixture | Expected behavior |
| --- | --- |
| `alloc_foundations_smoke.atto` | Successful exit 42; accounting, arithmetic and logical scrub assertions |
| `alloc_foundations_negative_secure_wipe.atto` | Non-success: guaranteed zeroization unavailable (must fail closed) |
| `alloc_foundations_negative_closed_read.atto` | Non-success: use-after-close rejected |
| `alloc_foundations_negative_stats_budget.atto` | Non-success: malformed budget rejected |

Negative tests must be correlated with the **intended error**, not merely any
nonzero compiler exit. In particular, unresolved imports cannot be counted as
passing runtime-denial tests. A source-only grep, local host model or a
zero-step Actions failure is **not** guest execution evidence.
