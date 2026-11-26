# RS — controlled dynamism / AOT / zero-GC review policy

**DRAFT / target guidance; not an implementation receipt.** See [organization-wide policy PR #38](https://github.com/ores-truffle-oreslang/.github/pull/38) and [specification PR #18](https://github.com/ores-truffle-oreslang/attolang-language-spec.md/pull/18).

## Compiler review requirements

- Declare runtime profile: interpreter/JIT/hybrid; host AOT; sealed full guest AOT; separately admitted precompiled-module AOT. No sealed profile can silently run guest `eval`, runtime AST/code emission, class generation, unrestricted reflection, unknown proxy targets or fallback to an interpreter/JIT.
- Generate virtual method tables, bounded interface/class proxies (separate static facades), generic/macro specializations and closure/async state machines ahead of time. Preserve visibility, type and effect checking, receiver identity, ownership and tree-shake reachability.
- Enforce explicit move/copy/borrow/share, actor heap and task frame ownership, fallible bounded allocation, cycle/long-lived actor reclamation, cancellation/I/O cleanup and generation fences. No closure/function messages or raw pointer actor escapes.
- Preserve language restrictions: no implicit conversions, operator overloading, `%`/`^`/`^^`, or implicit `Option`/`Future` flattening. Legacy/experimental divergence is labeled and rejected in canonical strict profiles; do not claim parity from transpilation success.

## Backend-specific qualification

Rust implementation does not automatically prove AttoLang guest borrow correctness or zero-GC; a Rust host can still use Arc cycles or leak arenas. Prove source-level checks and actual compiled .atto guest execution, distinguish Rust host tests from guest binary evidence, and reject unsupported runtime dynamic constructs before IR.

## Minimum fixture/evidence matrix

| Accept and execute | Reject before IR / stress |
| --- | --- |
| Known-class proxy and finite dispatch table, static facade, typed generics | eval, unknown class, reflection escalation, missing method reachability, runtime injection |
| Owned closure and task continuation, actor Sendable transport | escaped borrow across `await`, closure in channel, shared cycle without lifetime plan |
| Long-lived actor churn and deterministic teardown | quota/OOM, restart/cancellation while I/O active, stale generation, double free |
| Actual emitted target guest artifact and runtime output | host simulation, parser only, missing compiler, hidden runtime fallback, zero-step CI |

For claims of implementation, include exact PR head/tree, fixture blob SHA, compiler and artifact digests, runner/job step logs and memory/GC instrumentation. `steps=[]` or `steps=null` is **NOT_EXECUTED**. This page documents a target; it does not certify current compiler support.
