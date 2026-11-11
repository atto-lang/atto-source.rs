# Canonical Rust checker and native compiler passes

## Scope of the reconciled cyclic model

`cyclic_v1` preserves the unique field-sensitive, cyclic ownership/loan dataflow
from PR #2 alongside the merged V0 cleanup/MIR/shadow implementation. It is
**not** an AttoLang parser, production integration, complete NLL checker, full
AOT verifier or proof of guest execution. The original V0 and MIR V1 remain
unchanged apart from documentation/module registration and still reject loops.

Native Java, .NET, LLVM, Forge, Wasm and Cranelift compiler passes stay local for speed. They should reuse the same typed MIR graph while resolving types, effects, place identities and ownership, then perform CFG checking before lowering. A compiler may safely reject more programs than the Rust oracle; it must never silently accept a program the normative rules reject.

## IR invariants

- Place identity is (root, ordered field projections); sibling fields are disjoint, ancestor / descendant places overlap.
  The producer must prove nominal layout disjointness. This is not a model for
  arbitrary pointers, dynamic array indices, unions, overlapping slices or
  user-supplied field IDs. All roots start initialized; allocation and cleanup
  effects require a richer contract before compiler emission.
- Compiler-minted LoanIds are globally unique per function CFG. Two shared loans can overlap; exclusive loans cannot overlap any live loan.
- MAY-moved / MAY-loan state is unioned at joins; MUST-moved / MUST-loan state is intersected. A read is denied if **any** path moved the place. Reinitialize requires an exact definite move on **every** path.
- Loan end / use requires the loan to be definitely active. Every terminal block must have released all loans. Explicit EndLoan instructions are required until a real NLL lifetime pass emits them.
  `ReadThrough` accepts either loan mode; `WriteThrough` requires an exclusive
  loan. `UseLoan` alone does not grant write access. Raw owner access cannot
  bypass an overlapping loan.
- Suspension with any possibly live loan is rejected. Future pinned-frame support needs a separate proof of lifetime, frame retention and cancellation cleanup.
- Transfers are whole-root only and require authenticated Sendable-like evidence, and cannot overlap any loan.
- The frontend is trusted for type / effect facts in this preliminary API; a forged is_copy or is_transferable bit **invalidates any safety claim**. Strict mode must independently validate provenance and prove a complete mapping from real typed MIR and emitted operations.
- Unsupported operations (FFI escape, closures, reborrows, destructors with effects, partial moves with custom Drop, actor critical regions and abnormal exits) must be **rejected by adapters**, not omitted. A synthetic graph alone is not guest proof.
- Bounded graph size, projection depth, and dataflow work guard resource exhaustion. Malformed/orphan CFG nodes fail closed.
  Root/loan zero IDs and duplicate edges are rejected. All vector-length budgets
  are preflighted before operation scanning. The queue has at most one pending
  entry per block. Work charges include state/loan scans and joins, not merely
  iteration counts; retained incoming-state facts and transient join clones are
  separately bounded. Budget exhaustion returns an error, never a partial report.

## Executed local integration evidence

The combined debug suite executes **108 Rust library/model tests**: 70 existing
V0/MIR/shadow/budget tests and 38 cyclic-model tests (21 inherited, 17 added).
The added cases exercise read-only versus exclusive loan access, ended-loan
denial, repeated loan scopes, loop use-after-move, move/reinitialization loops,
late predecessor joins, nested/sibling projections and independent work/memory
budgets. Strict Clippy passes after replacing the inherited complex return tuple
with a named validation result. Release/format results are recorded in the PR.

This does not certify any backend's borrow checker. The cyclic model has no
cleanup-order proof, typed-MIR completeness envelope, shared-region promotion,
authenticated actor-domain lease, singleton, symbol or runtime revocation model.
Do not feed its success into V0's `Proof`, or replace a compiler rejection with
success from a lossy translation. These gaps remain required integration work.

## Required follow-up contracts

1. Versioned schema with source-unit, function, type-layout, alias provenance, effect, owner-domain, actor generation, loan and instruction identity. Bind every operation to compiler-produced typed MIR.
2. Exact source->CFG inventory and CFG->emitted code forward/reverse verification, including exceptional, cancellation and drop edges.
3. Nonlexical lifetime region inference, reborrows, two-phase borrows, branch-local loans, mutable container access and pinned-frame suspension.
4. A fixed shared conformance corpus with positive/negative guest sources, diagnostic codes and provenance receipts. Compare the canonical checker against Java/C#/C++ fast paths on identical IR.
5. CI must retain exact source/head SHA, Rust version, test steps, artifacts and provenance. No parser-only, synthetic, zero-step or host-only checks may be called certified compiled AttoLang guest results.

Related work: LLVM #1556/#1557, conformance #614/#615/#522, Java #2499 and .NET #419.
