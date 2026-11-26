# Incremental compilation v1

Status: **draft, first executable slice** (Rust compiler library).
Feature 5 of the 15 "additional features" proposals.

## Goal

Speed up large-project builds and editor feedback without ever serving a
stale artifact. Cache hits must be provably equivalent to recompiling.

## Shipped in this slice (`src/incremental.rs`)

| API | Purpose |
| --- | --- |
| `build_key(entry, units)` | SHA-256 build identity covering the compiler version, ABI version, target OS, entry, and every supplied unit's path, package and source digest. Fields are length-prefixed and units sorted, so the encoding is injective and order-independent. |
| `IncrementalCompiler::compile` | Bounded (1–4096 entries, FIFO) cache over `linker::compile_linked`. A hit returns the byte-identical guest **or the identical diagnostic**. |
| `IncrementalCompiler::changed_since_last` | Package-identity and content-digest diff (edits, additions, removals) since the last call. No mtimes. |
| `ImportGraph::affected_by` | The changed files plus every transitive importer: the exact set whose diagnostics an editor or language server must refresh. |
| `ImportGraph::reachable` | Units reachable from an entry. |

### Soundness note found while testing

`compile_linked` rejects unreachable units (`E_UNBOUND`), so its result
depends on *every* supplied file, not only reachable ones. A key limited to
reachable files would therefore let a hit return a guest where a fresh compile
returns an error. The key covers all supplied units, and a regression test
pins this behaviour.

## Tests

`tests/incremental_build.rs` (12 cases, real `compile_linked`): byte-identical
hits, invalidation on any keyed byte change, package identity in the key,
order independence, cached diagnostics, the editor refresh set, change
detection, bounded eviction, and fail-closed missing entry and duplicate paths.

## Next slices (proposed)

1. **Interface fingerprints with early cutoff:** hash each module's exported
   signatures (pub fnc/class/actor shapes) separately from bodies. A body-only
   edit then re-checks only that module, while importers keep their checked
   results. Prerequisite: per-module typed IR artifacts.
2. **Persistent cache:** an on-disk store keyed by `BuildKey`, written
   atomically (temp file plus rename), size-bounded, and holding no
   executable code that is not re-verified on load.
3. **attoc/LSP integration:** `attoc --watch` and the language server reuse
   one `IncrementalCompiler` and refresh only `affected_by(changed)`.
4. **Cross-backend:** the same key encoding (domain `atto-incremental-build-key-v2`)
   for Java/LLVM, so a shared CI cache can be addressed by any toolchain.

## Current-main reconciliation

The process-local key includes target OS; compilation pins that value on the synchronous compiler thread. Borrow-report mode bypasses the cache so evidence is not suppressed. Error cache hits require the same input order because the linker can choose the first invalid input in caller order. Package identity changes now invalidate editor refresh even when file bytes are unchanged. This remains a whole-build in-memory cache, not namespace-selective codegen or a persistent cache. The shared durable cache envelope in `compiler_cache_key.rs` remains separate. New-head execution is required; old local receipts do not qualify the merged candidate.

### Exact-hit work avoidance

The in-memory cache hashes the bounded complete input inventory before graph admission. Exact previously admitted hits skip repeated import-graph parsing; misses still run the original graph validator. The `graph_validations()` counter distinguishes this work from linker-internal parsing. The public `build_key` API retains full validation. Invalid inventories preserve existing diagnostic precedence; borrow evidence still bypasses reuse. No wall-clock improvement is claimed until benchmarked.
