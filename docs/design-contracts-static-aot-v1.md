# Compile-time-proved Atto design contracts: restricted Rust/Wasm v1

**Status:** real source frontend change; **tests not yet executed on an admitted runner.** Normative target: [atto-spec #211](https://github.com/atto-lang/atto-spec/pull/211). This first implementation slice deliberately does **not** claim dynamic design-by-contract, runtime `ContractViolationError`, actor supervision, or cross-backend parity.

## Exact compiler behavior

The production Rust/browser parser recognizes `requires predicate;`, `ensures |result| -> predicate;` between a named callable's return/effect header and body; class/actor members may declare `invariant predicate;` (actor: `invariant |s| -> predicate;`). Maximum 64 callable clauses; contract words are reserved and cannot become ordinary identifiers. This also applies before every alternate browser compiler fast path can publish Wasm.

Only a **syntactically proven** pure Boolean `true`, or one or more unary `!` applied to `true`/`false` resulting in `true`, can be compiled. They are erased because their truth is a sound static proof; the guest is otherwise unchanged. Constant `false` is rejected with `E_CONTRACT_ALWAYS_FALSE`. Obviously non-Boolean scalar predicates and bare non-Boolean parameter/result binders produce `E_CONTRACT_NOT_BOOL`. Unknown or dynamic predicates — even a plausible Boolean parameter, field or function call — produce `E_CONTRACT_UNSUPPORTED` **before any guest artifact**; no backend can silently elide them or convert them into assumptions. Invalid result binders and invariant placements are rejected.

```atto
fnc main() -> int
  requires true;
  ensures |r| -> !false;
::
  return 42;
end
```

The example is an **AOT candidate test**, not evidence that `cargo test` or Wasm guest execution has passed.

## Deliberate gaps

- No `requires value > 0`, `ensures |r| -> r > 0`, class state invariant or actor critical-write invariant execution. These are **not** compiled. Dynamic contracts need checked `bool` typed IR, transitive effect/loan analysis, CFG instrumentation for every successful and `?` return, cleanup-aware panic on a separate control channel, and a typed `ContractViolationError` with actor supervisor delivery.
- No claim of generic methods, full out-of-line `mtd` syntax or return-first `::` compatibility until those independent parser features land; this changes neither keyword spellings nor return semantics.
- No runtime flags to disable checks. No interpreter, reflection, JIT, runtime code generation or fallbacks. The compiler does **not** treat unchecked predicates as LLVM/Wasm optimization preconditions.
- No class/actor invariant AST or runtime metadata survives for the **proven-true** case; that is intentional static elimination, not proof of lifecycle or runtime invariant checking.
- No ordinary `std.assert` changes. That separately proposed API stays `import module assert from pkg "std.assert"; assert.true(...)`.

## Verification

`cargo test --locked --test design_contracts_static_aot` is the focused **real Rust compiler** suite. It checks successful Wasm emission for true clauses, parser-only actor/class cases, strict failures for false/non-Boolean/dynamic predicates, misplaced binders, and invalid headers; all failures must produce **no output artifact**.

The workflow `.github/workflows/design-contracts-static-aot.yml` pins checkout to current PR head, checks working-tree and blob SHA, executes the focused Rust suite, and explicitly excludes real Wasm guest execution from its claim. Until admitted CI runs, report **NOT_EXECUTED**. If source-org Actions are blocked before steps, run the exact SHA/tree via an authorized private alternate org using a token-free Git object mirror with per-blob verification. Do not treat source-only checks, a zero-step run or partial mirror as a passing compiled guest.

## Next compiler slice

1. Attach bounded, source-spanned contract expressions to the typed AST/IR, with result binder scope and whole-call-graph purity.
2. Emit contracts on the callee side; preserve return identity and check all CFG normal exits, including early `?` and `return`/`^`.
3. Implement typed non-catchable `ContractViolationError` panic with actor CONTROL supervision and deterministic cleanup, then executed Wasm AOT guest and compile-negative receipts.
4. Extend formatter round-trip and independent Java/LLVM/.NET/Forge/Wasm backend parity without weakening the full guest AOT rule.
