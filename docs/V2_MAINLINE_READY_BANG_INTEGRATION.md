# Experimental actor READY and checked bang — mainline integration

**Status: DRAFT / NOT EXECUTED.** This is a narrowly scoped, conflict-free
adaptation of [Rust PR #262](https://github.com/atto-lang/atto-source.rs/pull/262)
to the current `main` compiler, not a claim of complete parity or a replacement
for the existing public browser compiler.

## Why a new main-based integration branch?

The older v2 PR is more than 800 commits behind `main`. The current `main`
does not have `src/v2/`, and its lexer has removed `&&`, `||`, and `^^`
operators that the historic v2 parser referred to. A blind rebase or wholesale
replacement of `src/lib.rs`, `src/lexer.rs`, `src/parser.rs`,
`src/wasm.rs`, or `src/bin/attoc.rs` would discard newer ownership,
linked-module, type and package checks.

This branch preserves those current files (aside from a crate-local
visibility change needed to share already-identical checked arithmetic
helpers and a separately selected CLI profile), introduces the v2 modules
as a sidecar, and adapts v2 to **reject** the removed lexer operators. It
does **not** automatically try v2 when the default parser fails.

## Explicit opt-in

```sh
cargo test --locked --test v2_mainline_admission --test v2_actor_ready_guest --test v2_actor_bang_guest
cargo build --locked --bin attoc
target/debug/attoc --experimental-v2 tests/fixtures/pass/232-v2-direct-actor-pipe-bang.atto -o /tmp/direct.wasm
node tests/node-v2-ready-bang-guest.mjs
```

`attoc file.atto`, `attoc --check file.atto`, `attoc --package ...`,
`attoc --package-root ...` and the exported Wasm compiler ABI still use
the **current production frontend only**. No experimental fallback is
enabled for default users, package manifests, `/play`, or public releases.
The experimental CLI path is deliberately **single-source only**, with no
external imports, package discovery, or filesystem capabilities granted
to guest Wasm.

## Candidate semantics and proof boundary

- `spawn A()` registers an ActorRef; `await spawn A()` yields that ref
  after READY, not DONE. The existing guest actor constructor has only
  synchronous registration, no asynchronous startup hook.
- `actor.get_ready()` and `actor.get_done()` are distinct zero-argument
  Future-returning methods, with distinct runtime storage.
- `producer pipe! consumer` and `consumer rpipe! producer` are alternative
  spellings for one checked unicast DATA edge. The v2 guest's atomic
  `Result<PipeHandle,PipeSetupError>` helper denies duplication and type
  mismatch before publishing an edge.
- `actor pipe! bounded_channel pipe! actor` and its reverse spelling
  use the existing typed three-stage atomic guest admission helper.
- `stop! producer` gracefully closes accepted DATA ingress and does not
  force termination or settle either DONE Future immediately.
- These features run in **one guest Wasm memory/scheduler domain only**.
  No authenticated cross-TenantVM ActorRef, generation-safe process boundary,
  supervised CONTROL, native hard-real-time reclamation, or production
  PipeHandle cancellation is established.

The five new `.atto` fixtures `230`–`234` are compiled by the real
native `attoc --experimental-v2` in the new Node guest test. The test
instantiates the emitted Wasm, limits its imports to stdio print functions,
expects ordered `40\\n2\\n`, `main() == 42`, panic code zero, and
verifies the **default** compiler still rejects experimental source. New
Rust negative source-gate suites cover malformed bang syntax, nominal port
mismatches, unawaited/ill-typed lifecycle operations, recovery after
rejection, source limits and removed operators.

## CI, security, and release gates

CI is configured to run these tests in addition to current mainline
`cargo test --locked`, Clippy and existing browser-suite steps.
**No exact-head cargo compilation, Node guest execution, or Chromium
compiler/guest execution has passed yet.** A source-free Actions canary
in this repo failed before runner allocation. A workflow with zero steps
is `NOT_EXECUTED`, not a guest pass.

The source's authoritative
`.github/private-ci-approved-repositories.txt` on `main` lists **no**
approved cross-org private-source destination. Do not mirror code to any
other organization without an independently reviewed exact destination
approval; historical carrier minutes do not authorize export.

### Executed source-free private runner admission

A **source-free** capacity check succeeded on a candidate carrier:
`22-factor-apps/atto-ci`, numeric repository ID `1459040433` (private).
[Workflow run 44860823730](https://github.com/22-factor-apps/atto-ci/actions/runs/44860823730)
at the source-free probe commit `b314ac3f9b30ec6a4f28d4d9df5ca57e47117412`
executed the Linux environment and compiled/ran an original, standalone
Rust 42 smoke program with all steps reporting success. The source-free
snapshot's exact Git tree is `f5f57779e48939aeee3cdae825aa98924d9b2654`.

This establishes **runner capacity only**, not compiler behavior, available
budget for a full suite, or approval to export any private files. The
source repo's allowlist is still empty. Before transferring any compiler
source, a separate reviewed source-owned allowlist change must explicitly
authorize the exact carrier ID, scoped purpose, exact source commit/tree,
file exclusions, retention and cleanup, with independent security approval.
Do not infer permission from repository administrator access or this
source-free green job.

Before advancing toward `atto-lang.github.io/play`:
1. Reconcile overlap between old v2 PR #262 and lifecycle PR #25, directed
   pipe PR #224 and bang parser PR #450. This sidecar does not rewrite
   their branches or close them automatically.
2. Run the complete source CI, including actual experimental native
   compilation and executable Node guest tests; resolve real failures.
3. Reconcile async runtime with current package and ownership policies,
   then separately implement an audited, explicitly opted-in browser ABI
   and Chromium guest proof.
4. Complete independent reviews and verified immutable compiler SHA/digest,
   and only then promote supported `/play` previews.

This branch should remain Draft until steps 1–3 are supported by real
nonempty CI logs. No merge or public deployment is authorized from a
source-only or static result.
