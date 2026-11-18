# Stage 2d: external SSA diagnostic handoff (issue #271)

The Atto browser compiler now has a **host-only, opt-in** executable
`atto-loan-cfg-export` that compiles real Atto source using the existing
production checker, re-types the identical input and runs the bounded
`loan_cfg_oracle_v1`, then exports all supported typed-CFG nodes.

The versioned `ATTO_LOAN_CFG_V1` text stream includes every *admitted*
function, entry point, typed CFG instruction point and successor, loan issue,
read/write marker, NLL/lexical end, typed borrowed-load loan and owner,
direct owner reads, conservative trap/conditional flags and owner stores.
Rows are not supplied by user source; no codegen permission follows.

The independent consumer is implemented in
`ores-truffle-oreslang/atto-borrow-checker.rs` as
`atto_compiler_cfg_bridge_v1::analyze_manifest`, calling its existing
`polonius_ssa_v1::analyze_ssa` implementation rather than duplicating it.

## Use

```sh
cargo run --locked --bin atto-loan-cfg-export -- program.atto > checked.cfg
# From the separate canonical borrow-checker project:
cargo run --locked --bin atto_cfg_ssa_check -- checked.cfg
```

The reader's positive verdict means **diagnostic consistency only**. The
compiler's normal `compile()` behavior is unchanged. Never use the checker
to override a production error or authorize Wasm, AOT or actor execution.

## Limitation

The exporter runs production compilation and checked type lowering from
the same source input in separate deterministic passes. The diagnostic
manifest is not a cryptographic source/typed-MIR/Wasm transcript or
independently authenticated emitted-operation inventory. Incomplete
compiler lowering can still omit facts from both paths.

The current closed profile accepts straight-line scalar loans and
branch-local lexical loans with identical active state at joins. Unknown
loops/backedges, SSA phi merging, unproved checked-arithmetic traps,
exceptions, cancellation, heap, FFI and actor boundaries fail closed.
The wire format contains no phi operands; the separate SSA checker
validates definitions/dominance only for the admitted no-phi subset.

For CI, build source compiler and checker separately from exact SHA-bound
private Git subtrees, then generate a manifest with the actual host compiler
and consume it using the canonical independent Rust SSA implementation.
Assert positive/negative guest behavior and never claim cross-backend parity
from a native-vs-Wasm browser-compiler-only test.
