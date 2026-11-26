# Declared actor message contracts — Rust/Wasm frontend slice

**Status:** implementation draft, not verified guest conformance. Normative
semantics are tracked by [atto-spec #28](https://github.com/atto-lang/atto-spec/pull/28)
and the cross-backend cases by [atto-platform-conformance #33](https://github.com/atto-lang/atto-platform-conformance/pull/33).

## Accepted declaration shape (bounded subset)

```atto
define actor Router receives int sends int as
    susp receive(ActorMail<int> mail): void {
        stdio.println(mail.value);
        return;
    }
end

pub susp fnc main(): int {
    val router = spawn Router();
    router.send(42);
    return 0;
}
```

- `receives T` declares the input application payload type and must equal
  the existing `receive(ActorMail<T> mail): void` payload.
- `sends U` is an outbound application-message type **effect**, never
  a count or a destination declaration. An omitted clause remains the
  legacy implicit-effect mode.
- A `susp` marker is admitted on the illustrated handler and top-level
  callable only within this frontend's existing non-suspending executable
  subset. The parser must still fail closed on unsupported actual suspension.
- `ActorRef<Router>` retains its nominal actor identity; its `send`
  argument must match `Router`'s checked mailbox payload.
- A declared `sends U` checks typed `Send` expressions transitively through
  statically resolved helper calls, including branches not taken at runtime.
  Unknown call graph targets are rejected; there is no dynamic invocation
  widening in this subset.

## Explicit limitations

**This frontend's actor mailbox codegen currently supports only `int`
and `bool`.** In particular the requested
`receives string` + `ActorMail<string>` sample must produce
`E_UNSUPPORTED_TYPE`, not a runnable module. True string support requires
a validated length-delimited, ownership-safe message wire crossing worker
memories, host admission and guest decode with bounded quotas.

This restricted browser backend also rejects spawning actors *from within*
an actor, and rejects actor-reference types in actor method parameters.
Therefore the outbound effect analysis is a preventive type gate, **not**
proof of successful actor-to-actor outbound guest execution. It must be
independently exercised as the supported actor-ref and runtime subset expands.

`sends never`, union payloads, generic types, actor reply streams and
cross-isolate/remote serialization need their separately versioned support.
Supervisor priority CONTROL is runtime-authenticated and excluded from both
application clauses. No runtime topology, fan-out count or delivery ACK
guarantee is inferred from the declared types.

## Candidate UTF-8 DATA framing primitive (not connected to guest)

`src/actor_string_mail.rs` supplies a bounded, strictly validated host-independent
frame for future `ActorMail<string>` admission. The exact byte layout is:

| Offset | Field | Encoding |
| --- | --- | --- |
| 0–3 | Magic | ASCII `ATAM` |
| 4 | Version | `1` |
| 5 | Application payload kind | `1` = UTF-8 string DATA only |
| 6–9 | Payload byte length | unsigned 32-bit little-endian |
| 10 onward | Payload | exact, validated UTF-8 bytes |

A frame is limited to **65,546 bytes** including the ten-byte header; zero-length
strings are valid, while mismatched lengths, trailing bytes, wrong tags,
malformed UTF-8 and forged lengths must be rejected. The decoder returns an
owned `String` independent of the frame buffer. This deliberately grants **no**
mailbox, actor, tenant, sequence, or supervisor CONTROL authority. Distinct
Unicode byte spellings are not normalized or aliased.

This module is only a **candidate codec**, not a published network format or
source-level runtime ABI. It is not called by the Wasm compiler's actor send,
the Web Worker host mailbox, the worker memory copying bridge, or guest
`receive`. An acceptance of the original `receives string` source sample
remains **forbidden** until bounded cross-memory admission, guest decoding,
end-to-end ownership, and real compiled guest execution are implemented.
Do not reuse the separately proposed `Job<T>` envelope protocol as if it
were an `ActorMail<string>` DATA frame.

Standalone unit test (local Rust toolchain or authorized CI runner):

```sh
rustc --edition=2021 --test -D warnings src/actor_string_mail.rs -o /tmp/atto-actor-string-mail-tests
/tmp/atto-actor-string-mail-tests
```

**Evidence status: NOT_EXECUTED** until a trusted stepful runner produces
an exact-commit test receipt. Host codec tests do not prove guest transport.


## Host-side cross-memory DATA transport experiment

The accompanying `tests/browser/actor-string-bridge.mjs` implements a
**trusted-host** candidate bridge for the current Wasm
`actor_send(ref: i32, value: i64)` / `actor_receive(type: i32, value: i64,
sequence: i64)` calling convention. Packed string references are
`offset:u32 << 32 | length:u32` as an i64 and **may not cross memories
unchanged**. The bridge takes a byte-copy snapshot at admission, applies the
strict ATAM DATA frame, verifies same-tenant and minted actor capabilities,
enforces bounded FIFO/bytes, and copies only into a compiler-attested,
nonoverlapping receiver memory reservation. A return from the handler is
distinguished from admission, and a trap makes the receiver terminal.

Its API is intentionally **not wired to ordinary guest actor imports**.
The compiler currently has no certified allocation of the reserved receiver
window, no per-actor protocol metadata in its emitted ABI that a host can
authenticate, and no source-level ActorMail<string> lowering. The
`guestDataEnd` / `reservation` parameters in the standalone host tests
simulate a compiler-sealed memory layout; they are NOT a trustworthy
manifest for an actual untrusted guest. The host must not enable this bridge
for untrusted TenantVMs merely because a caller provided these values.

Run the host-level tests:

```sh
node --test tests/browser/actor-string-bridge.test.mjs
```

The tests use real WebAssembly.Memory objects and an independently assembled
Wasm export for the i64 pointer/length smoke test. The Wasm export is **not an
Atto-compiled guest**. Local Node 22.16.0 executed **12/12** tests on the
candidate host module; exact-source GitHub Actions for this revision still
require a nonzero-step, trusted run. Rust codec tests, compiled positive
ActorMail<string> guest execution, main-to-actor sends, actor-originated fanout
and authenticated supervisor CONTROL remain independently unverified.


### Checked scratch-page layout plan (not yet emitted)

The pure Rust module `src/actor_string_memory_plan.rs` plans the
host-receiver scratch page without silently widening existing guest heaps.
For the current Wasm frontend, scalar actor modules would use **2 fixed
pages** (ordinary 64 KiB + 64 KiB mailbox) and bounded heap actor modules
would use **17 fixed pages** (existing 16-page ordinary heap + 1 mailbox
page). The scratch starts at byte **65,536** or **1,048,576** respectively.
The plan rejects unsupported memory profiles, overlapping data/heap ranges,
mismatched Wasm memory limits and forged maximum page counts without
arithmetic wraparound.

This layout module is not yet invoked by the production Wasm emitter; its
presence must not change guest memory sections, mark string actors supported,
or grant a host permission. The next compiler integration must generate
independently attestable payload-type and memory-reservation metadata while
preserving byte-identical output for actors that do not receive strings.


### Exact-blob funded runner evidence (host + layout only)

[Run #44654596675](https://github.com/bait-bikes-test/atto-ci/actions/runs/44654596675)
ran real GitHub Actions build/test steps using an independent private
cross-organization mirror: **13/13 Rust DATA codec tests, 11/11 Rust
scratch-layout tests, and 14/14 Node/Wasm memory-bridge tests passed**.
All four mirrored source files matched their Git blob SHAs before testing.
Toolchains: rustc 1.99.1, Node 22.23.4. The carrier commit is
`0c0c6d92388be4940dbdb1918a176370de8c4314` and the verified source
head at mirror creation is `c77130f973aa0b667d6c7f13c63943617fd04018`.
The corresponding pin and explicit `NOT_EXECUTED` compiled-guest status
are in [atto-platform-conformance #33](https://github.com/atto-lang/atto-platform-conformance/pull/33)
(`evidence/typed-actor-string-memory-layout-host-20261119.json`).

This result proves the candidate codec, layout planner and host-side transfer
logic only; it does NOT enable source-level `ActorMail<string>`, execute an
Atto-compiled actor, attest a production guest memory region, or establish
multi-backend parity.

### Required compiler/runtime integration before enabling strings

1. Change the compiler only after allocating an actor-owned string mailbox
   region disjoint from read-only source data, checked heap and state. An
   untrusted guest must never be able to declare the host's reservation.
2. Publish a versioned actor payload-type/region manifest validated against
   actual Wasm sections, import/export signatures, memory limits, tenant
   ownership and the exact compiled artifact digest. Reject mismatches.
3. Type-check `ActorMail<string>` and `mail.value: string` in typed IR;
   preserve the existing destination's `receives` type and sender's
   `sends` effect. Preserve fail-closed behavior for unsupported unions.
4. Bind the host bridge solely inside the trusted actor supervisor's
   admission code, with real generation-fenced ActorRef capabilities and
   separate authenticated CONTROL. Never expose the bridge to guest JS.
5. Execute the exact source fixture from conformance #33 through compiled
   Atto -> real Wasm -> independent receiver memory, plus negative
   compile/no-artifact tests and stale-generation, full-mailbox, trap and
   cross-tenant denial receipts. Only then promote this backend.


## Verification gates

- A real pinned Rust compiler must build and compile the new positive fixture.
- The emitted Wasm guest must execute, printing 42, with exact module identity.
- The six negative fixtures must produce precisely the expected diagnostics
  with **no guest artifact**.
- CI/head SHA and toolchain must be recorded before claiming execution.
- Reconcile overlapping actor parser work in
  [atto-source.rs #297](https://github.com/atto-lang/atto-source.rs/pull/297)
  and the existing attached-receive
  [#216](https://github.com/atto-lang/atto-source.rs/pull/216)
  before merging.


## Actor isolation witness name hardening (source-only; no guest claim)

This branch now reserves `Sendable`, `Shareable` and `Serializable` as compiler-owned witness names in `src/parser.rs`. A guest's nominal class, actor or enum declaration cannot impersonate a transfer, freeze or format-specific wire proof. The restricted Rust/browser compiler rejects the unlowered names with `E_UNSUPPORTED_TYPE`, rather than minting an authority witness or implicitly assuming trusted codec support.

The new negative source cases `actor-isolation-pointer-mail-v1.atto`, `actor-isolation-arc-mail-v1.atto`, `actor-isolation-mutex-mail-v1.atto`, `actor-isolation-nominal-mail-v1.atto` and three `*-spoof-v1.atto` cases target a compile-error/no-emitted-artifact gate. The independent frontend still has a bounded int/bool actor-mail type subset. This change does **not** implement `Sendable<T>` MIR derivation, frozen `Arc<T>` runtime ownership, brokered cross-TenantVM transport or any new executable carrier. The general behavior is specified as a **proposal** in [atto-spec #27](https://github.com/atto-lang/atto-spec/pull/27) and staged as 36 unexecuted policy cases in [atto-platform-conformance #33](https://github.com/atto-lang/atto-platform-conformance/pull/33).

The conformance contract's separate **12/12 Rust source-only oracle** pass on the [private exact two-blob carrier](https://github.com/usa-acc/atto-ci/actions/runs/44662076919) is **not** evidence that this compiler branch, its seven new negative fixtures or Atto guest isolation passed. This branch requires its own exact-tree compiler build and checked negative diagnostics before Ready/merge.

## 2026-11-20 compiled Rust/Wasm actor isolation verification

**Executed, limited guest result:** [private GitHub Actions run #44770032043](https://github.com/ores-rate-limit-test/atto-ci/actions/runs/44770032043)
(job `136404901130`) built the production `attoc` native Rust compiler from
a private, token-free **68-file source subset** and compiled both
`actor-explicit-shared-domain-v1.atto` and `actor-bool-never-parity-v1.atto`
to valid Wasm. A minimal trusted Node host instantiated the compiler-produced
modules: `ActorRef.send(42)` executed the compiled receiving handler and
printed `42`, and a host-injected bool DATA event executed the compiled bool
handler and printed `true`. Test code:
`tests/actor_isolation_compiled_guest.mjs`.

The same exact-blob run passed **11/11 actual compiler integration tests**,
**5/5 compiler-owned scalar isolation witness tests**, **12/12 Rust affine
admission/dequeue tests**, and **9/9 negative `.atto` source compiler
rejections** with stable diagnostics and no emitted Wasm file. Rust toolchain:
`rustc 1.100.0`. All **68/68** source and fixture Git blobs were verified
against the *single source commit*
`9305b24573bbe0183dbbd6a59c8166c9f1635627` (tree
`e18e72ba95dd8ce7128a554527df2d67bdb4472e`). The private carrier
commit is `d2e6a0d44074abfc722a9e4589a65ae6bec5345e`.
Conformance evidence is recorded in `atto-platform-conformance` PR #33 at
`evidence/actor-isolation-compiled-rust-guest-20261120.json`.

**Do not misinterpret this proof.** The selected files include the complete
compiler `src/` tree, Cargo manifests and tested fixtures but are *not* the
complete 963-file Git repository. The synthetic host performs synchronous
trusted per-actor Wasm instantiation and DATA dispatch; it is **not** the
production actor scheduler, transactional runtime admission, an untrusted
TenantVM security boundary, capability broker, or cross-backend parity.
The guest `actor_send` ABI still returns `void`, and `take!` remains
compiler-rejected. No right to claim secure same-process multi-tenancy is
derived from this run. The PR stays Draft until independent review and
required exact-head gates are satisfied.
