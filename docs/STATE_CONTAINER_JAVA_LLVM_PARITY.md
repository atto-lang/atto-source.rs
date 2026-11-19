# Standalone protected state containers — Rust/Wasm vs Java/LLVM (2026-11-15)

**Status: restricted executable Rust/Wasm subset, NOT full Java/GraalVM ↔ LLVM guest-source parity.**
The source-org Java and LLVM main branches still reject or do not lower general
`define state container` guest code. Only the explicitly bounded browser subset
described here emits a Wasm guest. Do not promote the public /play compiler pin
from this document or from source-only validation.

## Grounded reference implementations

* Java: `oreslang-source.java/src/main/java/dev/oreslang/runtime/StateContainerAccessGate.java`
  provides non-parking read/write lease admission, writer exclusion, read concurrency,
  irrevocable close, active-lease checks and post-close quiescence. Its
  `ScalarStateContainer.java` stores **one long scalar** with checked addition,
  close and independently owned state, but is **VM-internal, not guest code lowering**.
  `parser/Parser.java` represents protected state as distinct
  `StateContainerDecl`; its general guest lowering is deliberately disabled.
* LLVM: `oreslang-source.llvm/src/main/cpp/oreslang/runtime/CriticalStateLoan.hpp`
  uses actor+continuation identities and a generation-fenced scope token, with
  cleanup at unwind. `docs/STATE_CONTAINER_REGION_RECLAMATION_V1.md` and
  `tests/state_container_virtual_actor_admission.cmake` distinguish native
  substrate validation from unsupported source compilation, and prevent IR
  emission for rejected source.
* Rust/Wasm: the current **narrow** backend lowers `define state container`
  (and Atto-only shorthand `define state`) to a private, instance-owned
  fixed-size Wasm memory allocation using the existing class heap backend.
  The AST preserves `ClassDecl.state_container` as a distinct nominal
  source kind. Typed `Stmt::StateWrite` checks actual write access, rather
  than inferring write authority merely from the field's mutable mode.

## Matched safety boundaries

1. **One sealed schema:** exactly one `state { ... }` block **before methods**;
   1–128 fields, **let** fields only, `int` or `bool`; only instance methods
   and no user override of `handle_state`. Field lookup from outside remains
   private. These limits deliberately do not claim all Java protected AST types.
2. **No implicit state references:** `self.count` from a state method and
   `counter.count` from outside are forbidden. `s` is a lexical capability,
   not a first-class value. Borrow escape, shadows and effects in critical
   blocks fail closed and produce no guest artifact.
3. **Read vs write:** `handle_state(critical-read |s| -> s.count)` projects
   a scalar **by value**. `handle_state(critical-write |s| -> { ... })`
   and `handle_state(critical |s| -> { ... })` permit checked scalar
   mutations only. Compiler typer requires a protected container and
   active `StateWrite` lease; nested scopes fail closed.
4. **No suspension:** calls, nested access, await, select, return, I/O,
   allocation and stored borrows inside a write callback are forbidden.
   This does not imply the full source language permanently bans these;
   the broader Java/LLVM effect-aware continuation design permits only
   operations for which the scheduler can prove loan retirement/readmission.
5. **Atomicity boundary:** within one Wasm guest instance the admitted
   callback is synchronous and non-reentrant, so checked scalar arithmetic
   and one field store complete without interleaving source instructions.
   Rust/Wasm implements **no cross-Worker/shared-memory RW gate**, no
   waiting queue or fairness, and no cancellation/close/reclaim API for
   state containers. These remain **not implemented**, not "equivalent".
6. **Memory lifetime:** separately instantiated containers have distinct
   heap addresses and initializers. They are still ordinary Wasm-owned
   objects under the provisional lowering, not a verified isolated
   actor heap, generation-fenced state region or portable ABI.

## Exact-source conformance requirements

Positive: short-form and Java-canonical long-form declarations, independent
instances, read/write/reset, checked arithmetic, boolean fields and
native-vs-wasm compiler byte parity. Negative: direct/unqualified access,
mutable read, escaping binders, stale/colliding aliases, duplicate schemas,
nested read/write, override of `handle_state`, unsupported types, effects,
empty state and over-budget declarations, with `E_*` diagnostics and
**zero guest Wasm**.

All findings must distinguish: **Rust library compile**, **Rust native
compiler**, **wasm32 compiler**, **actual compiled guest runtime**,
**Chromium Web Worker**, and **Java/LLVM source guest parity**.
A passing host gate or parser test never upgrades an unimplemented row.
A private test carrier may mirror source blobs via the connector only,
with SHA equality before running its own GITHUB_TOKEN and budget.
