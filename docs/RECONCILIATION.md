# Rust compiler reconciliation (2026-11-16)

`atto-lang` is the **single canonical home** for the AttoLang/Oreslang Rust and
Wasm compilers; `ores-truffle-oreslang` repositories point here.

| Path | Origin | What it is |
| --- | --- | --- |
| `/` (`src/`, `tests/`) | this repository | Browser compiler: `.atto` → guest Wasm, itself built for `wasm32-unknown-unknown` (playground) |
| `native/` | [ores-truffle-oreslang/oreslang-source.rs](https://github.com/ores-truffle-oreslang/oreslang-source.rs) `main` @ `e0b8124c69` | Cranelift native AOT compiler (`oresc`), MIR, effect/safety IR, managed-reference layouts |

`native/` was imported with a subtree merge (`git merge -s ours
--allow-unrelated-histories` + `git read-tree --prefix=native/`), so all 195
upstream commits are ancestors of this repository's history and no commit was
rewritten. At import, the `native/` tree hash equals the upstream tree hash
(`d22743b3201c53a392f4d8a08373f37e4c14e0f3`) except for one added file,
`native/rustfmt.toml`, which keeps rustfmt from inheriting the browser
compiler's root configuration.

CI: `.github/workflows/native-cranelift.yml` runs the upstream checks from
`native/` (upstream's checkout-identity step assumed the old repository root and
is omitted). Known pre-existing state carried over unchanged: upstream `main`
was not `cargo fmt --check` clean (25 files); clippy `-D warnings` and
`cargo test --locked --all-targets` pass. The inert `native/.github/` workflows
are retained as history only.

## Convergence obligations

The two compilers must not drift into separate languages:

- One normative grammar/semantics: [atto-spec](https://github.com/atto-lang/atto-spec).
- One conformance corpus: shared fixtures executed by both front ends, with
  unsupported constructs reported as UNSUPPORTED, never as success.
- One managed-reference model: actor-relative **32-bit** references on every
  host, matching the Java/.NET/LLVM heap32 contracts (`native/src/ref_layout.rs`
  vs the browser guest's private 32-bit linear memory).
- New Rust compiler work lands here, not in the Oreslang repository.
