# v2 guest actor READY, checked pipe bang, and graceful stop (candidate)

**Status:** staged on the experimental Rust v2 backend; no successful exact-head
native Rust build, emitted guest Wasm execution, or public compiler artifact
was verified on this branch. The current GitHub Actions source organization
has reported failures before runner allocation. This is not a production
TenantVM or multi-backend runtime parity certification.

## Source examples

```atto
pub suspends fnc main(): int {
  val producer = spawn Producer();       // immediate typed ActorRef
  val consumer = spawn Consumer();
  val ready_task = producer.get_ready(); // stable Future<void>
  val done_task = producer.get_done();   // DISTINCT Future<void>

  // Attach immediately; neither actor must first await READY.
  val Result<PipeHandle, PipeSetupError> link = producer pipe! consumer;
  // consumer rpipe! producer; describes the SAME directed DATA edge;
  // it is an ALTERNATIVE spelling, not a second connection or duplex.

  await ready_task;                      // resumes after actor initialization
  producer.send(40);
  stop! producer;                        // seal mailbox admission, do not kill
  await done_task;                       // actual actor completion after drain
  await consumer.get_done();
  return 42;
}
```

`val producer = await spawn Producer();` is accepted in a `suspends`
callable, returns `ActorRef<Producer>`, and waits for that actor's READY
Future (not its DONE). Existing guest actor constructors have only
synchronous field/mailbox/task registration: READY is settled after
registration finishes but before the constructor returns; an asynchronous
user startup hook is **not** claimed to exist.

The v2 guest actor record has a separate READY Future at offsets 56/64;
DONE retains offsets 32/40. Actor fields begin at offset 72.
`get_ready()` never closes a mailbox; `stop!` does not imply that
`get_done()` has settled. The actor loop drains accepted DATA, marks DONE
and propagates EOF to a connected downstream consumer.

## Checked bang transport

- `source pipe! sink` and `sink rpipe! source` compile to the same
  `__actor_link_try(source,sink)` Result-returning guest helper.
  Only explicitly declared, exactly matching nominal actor DATA ports are
  admissible. The helper checks fresh, open unicast endpoints, allocates
  the opaque PipeHandle and Result box, then publishes the edge in one
  cooperative turn, with no suspension or fallible operations after
  publication. Duplicate spelling returns Err and cannot attach twice.
- `source pipe! bounded_channel pipe! sink` and the reversed `rpipe!`
  triple compile to the pre-existing atomic checked three-stage helper.
  It enforces one producer, one consumer, bounded Channel capacity and
  no preloaded or competing consumers. Mixed checked/un-checked and
  mixed forward/reverse chains fail closed.
- Older `pipe` and `rpipe` experimental forms remain separately
  supported and may use legacy trap-based compatibility lowering.
  **Bang operators never silently lower to those trap paths.**
- `stop! actor` desugars to typed `actor.stop()`, which seals the
  actor's incoming channel. The cooperative actor loop finishes accepted
  messages and then settles DONE; this is **not** immediate STOP/kill.
  `get_done()` and pipeline completion are distinct concepts.

## Strict limitations and verification

This guest has one linear-memory/scheduler security domain. It does not
provide cross-TenantVM message transport, authenticated generation-fenced
capabilities across tenants, resource-loan reclamation, supervisor priority
CONTROL, production PipeHandle cancellation or hard real-time scheduling.
The browser Web Worker v1 host actor protocol is a *different* limited
execution profile, tracked separately under Rust PR #25. Do not combine
their result counts or claim guest parity from browser-host unit tests.

The following executable tests are staged, **not reported as passed**:

```sh
cargo test --locked --test v2_actor_ready_guest --test v2_actor_bang_guest
cargo build --locked --bin attoc
node tests/node-v2-pipe-guest.mjs
cargo rustc --locked --release --lib --target wasm32-unknown-unknown --crate-type cdylib -- -C link-arg=--max-memory=67108864
node tests/conformance.mjs
node tests/browser-e2e.mjs
```

Guest fixtures `230` and `231` exercise real nonterminal READY Futures,
`await spawn` and the checked bounded-channel bang forms. Fixtures
`232`, `233`, `234` exercise direct forward/reverse actor bang and
duplicate-edge denial. Each real Node guest test checks output `40\\n2\\n`,
`main() == 42`, panic code zero, and a restricted import allowlist.
No artifact is eligible for promotion to `atto-lang.github.io/play` until
the exact-head compiler build, live guest execution, independent review,
release digest and site security gates all pass.
