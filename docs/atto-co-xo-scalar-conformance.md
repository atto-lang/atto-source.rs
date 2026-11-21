# AttoCo / AttoXO cross-repository scalar compiler conformance

These fixtures snapshot the four `.atto` scalar projections from the open draft PR branches in atto-xo and atto-co. The source is copied verbatim before appending a runnable `pub fnc main(): int` test. The existing `tests/conformance.mjs` compiles every `tests/fixtures/pass/*.atto` fixture with both native `attoc` and Wasm compiler, enforces byte parity, and executes the generated guest Wasm with expected exit value 42. Each snapshot records its source Git blob SHA; refresh when the source branches change.

The test profiles are research-only. They do not establish production tenant authentication, durable leases, money movement or generalized language support. Merge requires the source fixture changes to be reconciled with source-project draft PRs and independently reviewed. No cross-organization workflow secrets or checkouts are needed: the fixtures are checked into this repo.
