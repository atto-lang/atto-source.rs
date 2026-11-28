# Backend-owned AttoLang std.net.udp — initial parity and negative tests

**Status:** pure `.atto` source candidates with tested source-model semantics only; NOT an installed executable imported UDP socket module. Each compiler repo owns its source independently; no dependency on a byte-identical shared stdlib.

## Files
- `stdlib/net/udp.atto` — pure IPv4/IPv6 port, length, path-MTU, receive/truncation, hop-limit and bounded queue helpers, including safe stream-packet accounting, release preflight, saturating drops and validated destination preflight.
- `stdlib/net/udp_stream_budget.atto` — stateful class candidate for packet+byte budgeting; **not independently compiler typechecked**. Do not claim this file compiles until genuine guest tests run.
- `tests/fixtures/udp-policy-main.atto` — 41 initial executable-in-principle boundary checks.
- `tests/fixtures/udp-boundary-adversarial.atto` — 41 additional positive/negative checks, including 64-bit overflow boundaries, full/zero-length packets and invalid family/port.
- `tests/fixtures/udp-stream-budget-main.atto` — 14 stateful candidate checks, awaiting compiler-backed class support.
- `tests/udp-policy-source-model.test.mjs` — 82 fixture evaluations, 2,048 deterministic independent differential inputs (5 assertions each), and mutation test.

Run local **host source-model only**: `node --test tests/udp-policy-source-model.test.mjs`. Its JS BigInt translation is NOT a real Atto compiler, Atto import, guest execution or AOT receipt.

## Proposed guest API (not yet linked)

```atto
import * as udp from "std.net.udp";
let socket = await udp.bind(udp.endpoint("127.0.0.1", 0));
defer socket.close();
await socket.send_to(bytes, udp.endpoint("127.0.0.1", 9000));
let datagram = await socket.recv_from(2048);
// datagram.bytes are owned and datagram.source preserves IP/port
```

`send_to`, `recv_from`, `recv_stream`, `close` and cancellation must use default async names without redundant `_async` suffix. A distinct blocking facade, if ever added, must use `_blocking`.

## Security invariants
No ambient socket access. Trusted host enforces scoped NETWORK permission on bind/send/receive and authenticates actor ownership and socket lifetime. DNS is not implicit; IPv6 scopes/jumbograms/multicast/broadcast need explicit design. Reject invalid port/family/payload before host work. Preserve one datagram per receive and zero-length packets, explicitly reject truncated packets. Enforce both packet and byte budgets before queue retention; overflow increments observable drops. UDP has no reliability, transport acknowledgements, backpressure on remote senders or stream-oriented byte framing. Use owned buffers, stale-generation checks, one-shot cancellation and no double close or cross-actor buffer aliasing.

## Verification frontier
Until an exact-head test actually compiles/links both the backend-owned `.atto` source and positive/negative guest fixtures and executes them in the target runtime, classify guest parity **NOT_EXECUTED**. CI failures with `steps=[]` are runner admission failures, not compiler evidence. Never export private Git source to a new organization without independent authorization, review and complete blob/tree SHA verification.

## Backend-specific status

Rust/Wasm compiler: no imported UDP guest socket provider or raw wasm32 UDP capability is installed. `attoc` must emit actual guest code for the two pure fixtures before support can be claimed. Host/network features require an independently authorized embedding; browser raw UDP remains unsupported.
