# Reconciled ownership baseline — 2026-11-10

This is an experimental **Rust library/model baseline**, not a complete AttoLang borrow checker or backend parity release.

## Preserved work

The integration retains the full commit history and unique work from PR #1 (bounded ownership CFG and cleanup), #4 (same-pass shadow callbacks and strict return terminators), #6 (buffer observations until whole-CFG success), and #5 (typed MIR inventory, global owner/loan identity uniqueness, pinned tooling and regression coverage). A semantic merge keeps both the strict return checks and typed-MIR/global-identity validation. The terminal check uses an iterator and saturating length arithmetic, including empty exits.

PR #2 remains a distinct experimental branch with field projections, partial moves/reinitialization and cyclic fixed-point analysis. None of that unique work is discarded or represented as included here. It requires reconciliation with cleanup ordering, operation inventory, provenance and the shadow API before becoming the canonical solver.

The added combined MIR/shadow entrypoint prevents callers from accidentally bypassing inventory admission to observe a graph. Preflight checks bound edge and per-block instruction vectors before cloning them. Seven additional regressions cover positive proof equivalence, incomplete inventory, unsafe ownership, observer failure, per-vector edges, cumulative edges and per-block instruction bounds.

## Executed local validation

Rust 1.99.1, macOS arm64: formatting, strict Clippy, all-target debug tests and all-target optimized release tests. The integrated suite contains **65 tests**: 25 MIR, 31 ownership, five atomic-shadow and four shadow tests; no ignored tests. The PR records the exact checked commit and results. No production guest, NativeAOT, JVM runtime, LLVM binary, sanitizer, or cross-backend differential result follows from these tests. Hosted workflow admission is reported separately.

## Refreshed compiler baseline

Remote default heads inspected on 2026-11-10:

| Backend | Commit | Observed integration gap |
| --- | --- | --- |
| Java | `6d8eb70048eafc9a84c5aebe9c1bb06104f16e76` | Existing Java `OwnershipChecker` has compiler-owned Task admission and root/loan checks. PR #2740 adds a partial shadow callback on a separate stack; it is not a complete typed MIR exporter. Preserve the local ownership-checker WIP when integrating. |
| LLVM | `0ee037ca3e5dd9ac6ac1e9cc3d40f19ee6326a9c` | C++ `OwnershipChecker`, `OwnershipCfg` and `OwnershipDropPlan` exist; source/emission inventories and typed loan/place correspondence must be reconciled with the Rust contract. |
| .NET | `3f3606cc3ef81dad117047b4cb57609ad1a92672` | Independent typed CFG/CIL compiler has a bounded source subset. General borrowed object lowering and a canonical ownership adapter remain outstanding; do not certify it from runtime host tests. |

Java had **1,186 open PRs** at inventory time. They require dependency-aware per-PR review; a title match or a shared test name does not establish parity. No bulk closure is justified.

## Completion gates still open

- Reconcile PR #2's field-sensitive loops with one solver API, including path-specific cleanup and exceptional control flow.
- Independently derive Copy/Send and canonical alias/place identities in each compiler, and census all ownership-relevant source and emitted operations.
- Define critical-state and shared-region operations, actor/domain generations, singleton/symbol ownership, cancellation and pinned continuation lifetimes. Unsupported operations remain rejected.
- Keep Java/C#/C++ native checks in the normal type/IR passes; CFG fixed points can still require multiple iterations. Compare them against Rust on the same supported cases before enabling fast paths.
- Any serialized cross-language bridge needs independently authored TypeSpec and JSON Schema Draft 2020-12 peers admitted by TJSV. This Rust API integration does not introduce or certify such a wire contract.
- Run shared `.atto` compiler-negative and executable-positive fixtures, plus native lifetime/concurrency tests, before claiming backend parity. Use the current AttoLang language spec, including the concurrent-default actor amendment.
