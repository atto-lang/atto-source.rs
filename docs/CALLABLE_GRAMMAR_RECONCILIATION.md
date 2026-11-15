# Callable body grammar reconciliation (Rust native/Wasm)

This integration branch takes **parser and target-only receiver syntax from**
`atto-source.rs` PR #147 at `9c1db7d22adccb19cb624ac15d91620d1f62deeb`,
and the independent positive/negative fixture matrix from PR #148 at
`c3e3e470dd67407959b783b21621d6ce4730e612`.
It is a temporary stacked verification branch, not a third competing
language grammar proposal.

## Accepted five styles

```atto
fnc f(): int { return 5; }
fnc g(): int -> { return 5; }
fnc h() -> int { return 5; }
fnc i() -> int
    let int x = 3, x + 2
end
fnc j() -> int ::
    let int x = 3, x + 2
end
```

The same body grammar applies to `routine` and inline class/actor instance
methods. In an `end` callable only, commas separate top-level items and a
final value expression is lowered to a typed return. Brace blocks retain
semicolon/newline syntax. Embedded argument commas do not act as body
separators. A trailing comma, unclosed `end`, `::` before a brace,
`: int ->` before an unbraced body and empty bodies must fail closed.

`mtd (self: Type) ...` out-of-line receiver bodies remain **syntax-only**;
`target_syntax` must reject missing delimiters, direct sealed actor state,
and unsupported effects. No callable ABI, closure capture, dynamic method
dispatch, actor method sync call or browser binary promotion is implied.

## Proof obligations

- `cargo test --locked` on exact source tree, including the combined Rust
  parser suites from both original PRs.
- Native `attoc` and actual `wasm32-unknown-unknown` compiler builds.
- `ATTOC=target/release/attoc node tests/conformance.mjs`: real guest
  execution for the added positive fixtures, exact native/Wasm bytes and
  diagnostic-only negative fixtures.
- `ATTOC=target/release/attoc node tests/callable_guest_execution.mjs`:
  independent execution and explicitly no artifact on rejection.
- `node tests/target_syntax_wasm.mjs` and actual Chromium worker E2E.
- Compiler source exact Git tree and blob SHA verified before trusting a
  private alternate CI carrier; no automatic compiler binary publishing.
