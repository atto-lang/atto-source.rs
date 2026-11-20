# Rust Oreslang parity & safety gates

**Product requirement:** The Rust compiler must eventually be as complete and correct as Java/GraalVM and C++/LLVM; performance should be benchmarked independently and improved per target rather than claimed by backend choice. Cranelift is the native code generator, not the ownership checker or language specification.

## Four-tier acceptance model

1. **Language admission:** identical Oreslang grammar, nominal/structural/ADT semantics, methods, generics, pattern matching, default arguments, modules, visibility, ownership and diagnostics. An unsupported language construct is a compile error, never a silently ignored effect.
2. **Runtime parity:** actors, actor groups, isolates/trust domains, per-actor heaps, zero-copy immutable shares, tasks, \`await\`/select/channel semantics, lifecycle, supervision, errors, exceptions, hot reload and ABI.
3. **Static safety:** every backend exports typed source-aware MIR/effect summaries to the independent analyzer, including CFG edges, mutation/borrows, failure paths, cost annotations, scheduler fairness, lock/channel dependency graphs, cancellation and FFI. Compare diagnostic classifications across implementations.
4. **Performance & target readiness:** compile latency, peak memory, binary size, workload throughput, p50/p99/p99.9 latency, pause budgets, CPU usage, run-queue fairness, cold-start and target-specific WCET evidence. Record baseline machine, toolchain, test revisions and confidence intervals before claiming any win.

## Non-negotiable safety analysis

| Contract | Rust analyzer admission requirement | Escalation |
|---|---|---|
| Move/borrow | Place/path-sensitive ownership graph, exclusive mutation, non-overlapping disjoint-field loans, no use-after-move, loan lifetime/outlives checks, actor heap isolation | Error on ambiguity for \`safe\` mode |
| Zero-copy sharing | Independent immutable/freeze proof plus allocation/trust-domain and generation checks; mutable cross-actor pointers forbidden | Error |
| Deadlocks | Lock order DAG plus scheduler/actor/channel/select wait-for graph, resource IDs, timer and cancellation edges; report cycles and unresolved waiting | Error/indeterminate; no universal claim |
| Cooperative scheduling | Detect loops/backedges, bound work between \`rt cooperate\`/yield points, compiler-inserted watchdog counters where permitted | Deny hard-RT without bound; warn soft-RT |
| Structured concurrency | Scoped Task lifetimes, join/cancel/quiescence, outliving borrows, unobserved Task errors, abandoned futures and leaked handles | Error |
| Errors | Effect obligations for Result/Option/\`trap\`/\`throw\`/\`raise\`/\`panic\`; catch/raise/supervisor propagation along every normal/unwind edge | Error |
| Soft real time | Bounded pauses, scheduler and allocation/memory pressure telemetry; explicit latency SLO, worker preemption and GC policy | Soft policy diagnostics |
| Hard real time | Verified allocation bounds, bounded recursion, stack, blocking/FFI contracts, schedulability, bounded preemption and target-specific WCET evidence | **Cannot certify from language-level IR alone** |
| Security | No undefined behavior/unchecked FFI, capability provenance, hot reload generation fencing, trust-domain enforcement, incomplete-IR rejection | Error |
| Size & optimizations | Reachability/tree shaking across core/alloc/std/user modules, no duplicate copies of code per actor, target-specific codegen and layout | Measured regression budget |

See the Rust analyzer's \`docs/SAFETY-GATES-V1.md\` for the **implemented initial** linear-effect admission pass and its proof boundaries. The source compiler's \`--safety-ir\` emits that format **only for the present scalar-only language subset**; \`call\` events are explicitly proof gaps. It does *not* attest to a completed borrow checker for general programs.

## IR invariants and pass ordering

\`\`\`text
lossless CST (Rowan, planned)
  → typed HIR + effect signatures (planned)
  → source-aware ownership/region CFG MIR (planned)
  → borrow + actor/trust-domain + Task/error checking (planned)
  → wait-for graph / cooperation / allocation + target policy admission (partial analyzer)
  → optimization/legalization with effect preservation (planned)
  → Cranelift IR (scalar native backend implemented)
  → object / link / runtime ABI (scalar object implemented)
\`\`\`

A pass may only discard a safety-relevant annotation when another verification pass guarantees the same property; compiler optimization must be validated with differential tests and metadata provenance.

## Rollout milestones

- **P0 (now):** keep seven canonical scalar native tests and five diagnostic-class tests green; compile+link+execute, never rely on parsing alone. Add Rust compiler → analyzer safety-export adapter, fail closed on unsupported call summaries. No new required external tokens.
- **P1:** full syntax/CST and typed HIR; SSA CFG verifier; ABI compatibility; signed integer semantics/overflow; calls/recursion and check-only parity; integrate Salsa incrementality after semantics stabilize.
- **P2:** sound ownership graph with field projections, move/borrow/await lifetimes, per-actor arenas and immutable shares; exception/unwind and cooperative scheduling MIR.
- **P3:** actor and Task runtime; actor group, supervision, send/ack, continuation frames, cancellation, select fairness; concurrency and deadlock graph audits.
- **P4:** soft/hard real-time target profiles, WCET evidence pipeline and scheduler integration, stack/memory/FFI resource bounds; freestanding/no-alloc core, alloc, std separation.
- **P5:** auto-generated cross-engine semantic/runtime/proof fixtures, performance suites, Oreslang self-hosting bootstrap with stage0 Rust → stage1 Oreslang reproducibility checks.

## CI merge policy

- Never mark a feature *supported* based on a design document or analyzer schema test.
- Require successful **native execution** where semantics depend on execution; error tests compare normalized classes/IDs.
- Every new AST node requires a tested MIR lowering and static effect contract.
- Missing budget, missing target proof, dynamic call, unchecked extern, missing CFG edge, partial analyzer coverage → **unknown/reject**, not \`safe\`.
- Full hard-RT certification also requires platform evidence external to any backend, not merely passing \`--safety-ir\`.
