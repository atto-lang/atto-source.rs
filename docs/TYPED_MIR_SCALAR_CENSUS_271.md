# Compiler-owned typed scalar MIR census (issue #271, stage 1)

**Status: restricted, opt-in, diagnostic-only slice. It is NOT complete borrow checking, full compiler-to-Wasm emission attestation, Polonius/SSA CFG parity, or a cross-backend safety gate.**

This stage builds useful **real** origin and use facts from AttoLang's **checked typed IR** rather than from manually authored external fact objects. It is stacked on [PR #273](https://github.com/atto-lang/atto-source.rs/pull/273), which already runs bounded scalar `borrow!` last-use checks on actual AttoLang source.

## API

`atto_compiler::compile_with_typed_scalar_census(source)` runs the same lexer, parser, type/ownership check and Wasm emitter as `compile(source)`. Between the checked typed IR and emitting bytes it calls `check::typed_mir_census::from_checked_program(&typed)`. Unknown effects fail closed **before** the optional audit path emits any bytes; the default production compiler retains all existing diagnostics and behavior.

For each supported function it records a stable, one-based `instruction_id`, source-function index/name, parameters' initial SSA origins, and each typed scalar assignment/return/expression's root origin **uses**, SSA new **definitions**, and straight-line successor. Assigning the same local again allocates a new origin; reads after a definition name only the current origin. Both pure ternary arms contribute conservative MAY uses. There are explicit resource limits for function/point/origin/use counts and expression recursion.

**Closed subset:** `int`, `bool`, `float`, straight-line scalar `Set`, pure literal/local/arithmetic/ternary expressions and explicit return. Only simple non-void/void scalar storage is accepted. Calls, actor/heap effects, object or field access, exception/cleanup, `await`, loops, branches, dynamic joins, shadow aliases, and other effects are **unsupported**, not silently dropped. Every unsupported operation returns `E_TYPED_MIR_CENSUS_UNSUPPORTED`.

## Critically: borrow issues no longer disappear during typing

Previously `bind_scalar_borrow` lowered an accepted `val reader = borrow! owner;` into a generic `TStmt::Block(Vec::new())`. The compiler now retains an explicit **`TStmt::ScalarBorrowIssue { owner_slot, alias, exclusive }`** marker. The Wasm emitter emits no bytes for the marker, as guest scalar aliases already lower to the original validated owner slot. The `checked_borrow_issues` diagnostic API reports issue sites from the actual checked typed IR without caller-supplied provenance.

**This is intentionally not yet a full borrowed-use census.** A borrowed alias read or early retirement is still lowered to ordinary scalar slot loads and cannot be reconstructed uniquely from typed IR. The strict `from_checked_program` interface therefore **rejects every borrow issue** rather than accepting it as an ordinary empty block or fabricating loan use/kill data. This distinction is mandatory before integrating the canonical Rust checker.

## Required stage 2 before any authoritative use

1. Carry checked loan IDs, distinct alias-use operations, NLL retirement, lexical close and correct cleanup edges through typed MIR. No user-provided origin facts; every guest instruction needs an independent lowering inventory and emission witness.
2. Add SSA phi and real CFG branch/loop dominance, tracked alias roots/projections, exception/cancellation, actor `Sendable`/transfer, and `await` pinned-frame safety. Unknown cases must continue to reject.
3. Differentially run the compiler-local checker and the canonical Rust experimental validator from [atto-borrow-checker.rs PR #41](https://github.com/ores-truffle-oreslang/atto-borrow-checker.rs/pull/41). Neither diagnostic result can override a production rejection.
4. Compile and execute positive and negative AttoLang guests under both native and Wasm browser compilers. Then independently prove cross-backend parity and source/typed-MIR/emitted-Wasm coverage.

## Verification commands

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
cargo build --release --locked --bin attoc
cargo build --release --locked --target wasm32-unknown-unknown --lib
node tests/scalar_nll_selected.mjs
node tests/ownership_wasm.mjs
```

If original source-org runners cannot execute, reconstruct the precise source Git tree in an authorized private Actions carrier via individually SHA-verified blobs; never transfer cross-org private tokens. Partial-tree carrier runs must be described as **partial**, not full-source CI.
