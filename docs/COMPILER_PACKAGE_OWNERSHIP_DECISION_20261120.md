# Rust compiler package ownership: #448 versus #455

**Status:** Draft integration decision, not an implementation, CI PASS,
release approval, or permission to merge (2026-11-20). The language spec
remains the authority for grammar and semantics.

## Selected package boundaries

- Root `Cargo.toml`, `src/`: `atto-compiler-browser`, exposing
  `atto_compiler` and `attoc`; native Rust host compiler and
  `wasm32-unknown-unknown` compiler that emits browser guest Wasm (ABI v2).
- `native/Cargo.toml`, `native/src/`: the independent Cranelift native
  `oresc` compiler and safety/effect IR prototype, currently named
  `oreslang-source-rs` until a separately reviewed crate rename.
- Root `tests/`: 88 browser integration suites explicitly registered
  by #448; 38 legacy native-only root test files **excluded** from browser
  Cargo. The dedicated native package already has 15 same-named suites:
  7 are byte-identical to their root counterparts, while 8 have different
  Git blobs. Native Cargo now explicitly registers **31 distinct root tests**
  under `legacy_*` target names, covering the 23 previously absent names
  **and all 8 divergent variants**; its existing 15 `native/tests/*.rs`
  suites stay auto-discovered. The structural inventory checker verifies
  root and native file-byte equality, test names, and paths.
  **Registration is not execution**: native Cargo tests have not run on the
  current head because source Actions is still failing before runner admission.

Keep the canonical browser crate at the root and Cranelift in `native/`.
Do not replace the root manifest with a native package or add a second
`browser/Cargo.toml` in #448. The structural gate
`scripts/verify-browser-cargo-tests.mjs` checks package ownership and the
explicit test inventory; it is not equivalent to building either compiler.

## Competitor disposition: PR #455

[PR #455](https://github.com/atto-lang/atto-source.rs/pull/455) proposes
the incompatible inverse layout: native Cargo at root and an older
browser compiler under `browser/`. **Do not merge the whole PR into #448.**
Distinct functionality must not be discarded:

1. **ESM:** `browser/src/js.rs` and `compile_js` emit JavaScript from
   an older checked IR. Evaluate a semantic port onto the *current*
   source/checker, or an explicit integration with `atto-source.js`,
   retaining one normative AttoLang grammar. Copying the file verbatim
   would not establish ABI/type parity. Require real ESM execution,
   negative diagnostics, deterministic output and no unchecked fallback.
2. **Actor guest/private memory:** compare #455's browser actor tests
   and `browser/src/wasm.rs` with current root `src/wasm.rs`,
   `tests/browser-e2e.mjs` and actor guest ABI v2. Port only uncovered
   guest/worker isolation, shutdown, failure or lifecycle behavior
   with real runtime receipts. Actor Wasm instance memory isolation
   does not by itself prove TenantVM security.
3. **Native changes:** review #455's CLI, native `.atto` example and
   effect/safety assertions against `native/`; keep unique linked
   executable cases rather than overwriting the production root.
4. **Style-only changes:** do not duplicate broad reformatting without
   measured benefit and an exact-head test run.

The **current disposition** of #455 is *competing and unresolved*.
Keep it available until each unique improvement is adopted and tested,
superseded with evidence, or explicitly rejected by maintainers.

## Executable acceptance criteria for #448

On an **authorized exact-head runner** run:

```sh
node scripts/verify-browser-cargo-tests.mjs
node --check scripts/verify-real-browser-compiler.mjs
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked --release --bin attoc
CARGO_PROFILE_RELEASE_OPT_LEVEL=z cargo rustc --locked --release --lib \
  --target wasm32-unknown-unknown --crate-type cdylib -- \
  -C link-arg=--max-memory=67108864
ATTOC=target/release/attoc \
  ATTO_WASM_COMPILER=target/wasm32-unknown-unknown/release/atto_compiler.wasm \
  node scripts/verify-real-browser-compiler.mjs
ATTOC=target/release/attoc node tests/conformance.mjs
node tests/browser-e2e.mjs
(cd native && cargo test --locked --all-targets)
```

Verify executed steps, current source/tree SHA, binary hash/size, guest
byte equality and execution, checked overflow, negative diagnostics
and **no stale executable artifacts**. Restore native coverage separately.

## Source-local exact-head CI admission (no source export)

If hosted Actions is not admitting source-private jobs, the **same** `ci.yml`
now supports an administrator-only operational path patterned after the
already existing [SelectPlan #470](https://github.com/atto-lang/atto-source.rs/pull/470)
source-local runner admission. This is an **unexecuted configuration option**,
not proof that a runner has been provisioned or tests have passed.

1. A repository administrator must approve and register a **dedicated,
   isolated, preferably ephemeral self-hosted runner in
   `atto-lang/atto-source.rs` itself**, with a unique private label, and
   configure repository Actions variable `ATTO_PRIVATE_RUST_RUNNER_LABEL`
   to exactly that label. Do not point it at a carrier in another organization.
2. First open **Actions → Source-free runner admission canary →
   Run workflow**, choose the current PR branch and enter the current full
   **40-character HEAD SHA** as `expected_head`. Require an actual assigned
   runner, recorded shell step and `SOURCE_FREE_RUNNER_ADMITTED` output.
   This canary has `permissions: {}`, no checkout, secrets, cache or artifact.
   A zero-step run is **NOT_EXECUTED** and must block the next action.
3. Only after the canary succeeds, open **Actions → Portable Atto frontend
   and browser Wasm conformance → Run workflow**, select the *same current*
   source PR branch, and provide the current full 40-character **commit
   SHA** as `expected_head` plus the Git commit's 40-character **tree SHA**
   as `expected_tree`. Fetch the tree from the verified commit object; do
   not infer it from source file names or branch labels. The job compares
   `expected_head` with immutable dispatch `github.sha` **before checkout**
   and checks the actual checkout tree against `expected_tree` before any
   code/test script runs. A mismatch fails closed.
4. Automatic `pull_request` and `push` jobs always use
   `ubuntu-24.04`, never the repository-configured private runner label.
   Checkout pins the head SHA with `persist-credentials: false` and
   `contents: read` only. The workflow retains the exact-head tree
   check, the source-local private-export deny policy test and guard, Cargo
   target inventory, Rust 1.85.1 native and wasm32 builds,
   `cargo fmt --all -- --check`, Cargo/Clippy, native-package tests,
   real compiler and compiled guest tests, browser E2E and receipts.
5. Independently inspect the actual run ID, steps, exit codes, generated
   compiler artifact, source HEAD/tree, and any required security review.
   A failed zero-step job, a synthetic-only canary, or a run on an earlier
   SHA is **NOT_EXECUTED**, not a release pass.

The variable and runner must be provisioned by a human administrator; the
GitHub connector cannot provision/approve runner infrastructure or enforce
private-main branch protection. A stepful test run **alone** cannot bypass
[#442](https://github.com/atto-lang/atto-source.rs/issues/442) or
[#446](https://github.com/atto-lang/atto-source.rs/issues/446).

## Security/release conditions

[Runner admission #440](https://github.com/atto-lang/atto-source.rs/issues/440),
[branch protection #442](https://github.com/atto-lang/atto-source.rs/issues/442)
and [private CI export #446](https://github.com/atto-lang/atto-source.rs/issues/446)
remain blocking. Empty-step CI failures are `NOT_EXECUTED`, never green.
The private source transfer allowlist is empty: no cross-org Git mirroring,
public artifact publishing, unauthorized bypass, Draft→Ready promotion
or merge until independently authorized tests, reviews and protection
are satisfied.
