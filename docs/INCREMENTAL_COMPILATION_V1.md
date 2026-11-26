# Incremental compilation design v1 — Rust-authored AttoLang source compiler and browser Wasm compiler

**Status: DRAFT DESIGN ONLY.** This commit does not implement new incremental code paths and is not compiler or guest-execution evidence.

Shared contract: [atto-common-source #133](https://github.com/atto-lang/atto-common-source/pull/133); rollout tracker: [atto-compilers-parity-testing #53](https://github.com/atto-lang/atto-compilers-parity-testing/issues/53).

## Required cross-backend semantics

- Within a resolved, immutable package instance, source files with an explicit `namespace N;` may contribute to a **single package-qualified namespace** spanning multiple files/folders. Namespace identity is `(PackageInstanceId, N)`.
- A source file **without any namespace header** is an **independent compilation/cache unit** keyed by `(PackageInstanceId, SourceUnitId)`. It never joins an implicit global/default namespace; modifying it cannot cause unrelated unnamespaced sibling files to recompile.
- On a private/body-only edit, recheck/regenerate the owning unit or a finer proven function unit; unaffected files/namespaces retain verified typed outputs. On public ABI/effects/ownership/actor-contract change, invalidate bound importers and negative lookups; inline, macro, specialization, constants and whole-program optimizations create additional edges.
- Package manifests/locks define membership and version provenance, **not** the minimum rebuild scope. Deleted/moved files, renamed namespace headers and package-version switches must purge stale graph entries. Cyclic dependencies use deterministic interface-first SCC processing.
- Cached work is never proof by assertion: rederive complete source membership, dependency resolution and source/ABI hashes from a trusted snapshot before reuse. Fail closed for unresolved imports, path escapes, unsigned/foreign/stale artifacts, negative lookup changes, partial outputs and missing ownership/borrow proofs.
- Target-specific code generation and final artifact relinking are separate steps. The build explanation must distinguish `RECHECK`, `CODEGEN`, `RELINK`, `REUSED`, `UNSUPPORTED`, and `NOT_EXECUTED` rather than hide a full rebuild.

## Repository entry points

`src/`, `tests/`, `Cargo.toml` and browser/Wasm compiler entrypoints.

## Baseline and overlapping work

The Rust compiler has an executable restricted AttoLang subset and can be built to wasm32 for browser compilation. Some namespace/out-of-line syntax is explicitly parse-only; it must not be marked compiled.

## Target implementation

Implement in-memory incremental query keys for source parse, name lookup (including negative resolution), namespace membership, typed semantic facts, borrow/actor admission, optimized IR and Wasm object assembly. For no-header files, the smallest safe source compilation unit is the file. Recheck only changed units and importers of changed public ABI. Reuse stable source maps and resolve a fresh complete package snapshot before any cache hit.

## Backend-specific cache boundaries

Separate host-native compiler and wasm32 compiler cache profiles; bind rustc version, runtime/guest Wasm ABI, target feature flags, memory limits and compiler semantic version. Regenerate final Wasm container as needed without claiming all source units were recompiled.

## First executable acceptance slice

Test actual native Rust compiler and wasm32-hosted compiler compiling the same `.atto` edit matrix; execute guest Wasm in Node/Chromium with clean-vs-incremental byte/effect comparisons and memory-lifetime negative cases.

## Safety and overlap

Do not mistake parse-only namespace grammar for semantic compilation or published playground support.

## Required verification before merging or marking supported

1. Add real source-level implementation and tests; machine-readable contract status remains `DESIGN_ONLY` until merged, exact-current-head, stepful compiler/guest evidence exists.
2. Run canonical **10 files / 3 namespaces + 2 unnamespaced files**: no-op; body-only edit; ABI/effect/ownership change; inlining; namespace move; deletion; duplicate/negative import; package pin change; target-profile change; corrupted cache; and cold-clean differential.
3. Prove changing just one unnamespaced file leaves another unnamespaced file's parse/typing/codegen counters and output hash untouched (aside from justified link work). Prove package-bound explicit namespace membership and unchanged-ABI callers are reused.
4. Compare incremental and clean diagnostics, emitted public interfaces, artifact reproducibility when promised, and **real compiled guest behavior**. Record exact Git SHA/tree, target/toolchain, nonzero CI steps, compiler output and exit statuses.
5. Never export private source to an unapproved CI carrier. If source Actions admission fails with zero steps, report `NOT_EXECUTED` and use only separately approved, exact-tree, per-blob-verified private CI mirroring. Independent review and branch protections remain mandatory.

Design/checklist files here are intentionally **not** a substitute for the above backend changes.
