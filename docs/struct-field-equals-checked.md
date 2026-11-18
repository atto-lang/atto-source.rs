# Checked equals-style field initialization for nominal structs

**Status: implementation candidate, not yet CI-verified.** This is a narrow compiler slice toward [atto-spec PR #84](https://github.com/atto-lang/atto-spec/pull/84). It is **not** a general named-call grammar change, general-purpose struct literal evaluator, generic array initializer, or SoA implementation.

## Supported executable profile

Only an explicitly declared file-local nominal `define struct` whose fields are all scalar `int`, `float`, `bool`, or `string`, currently admitted by `shape_lower`, may use `StructName(field=literal, ...)`. Fields must be present exactly once; unknown, duplicate and missing fields are compile errors. Each value must be one currently accepted scalar literal token. Both positive forms feed the **same checked existing class-constructor and Wasm emission path**:

```atto
define struct Particle as
  int x;
  int y;
end

pub fnc main(): int {
  val a = Particle{x: 10, y: 20};
  val b = Particle(x=10, y=20);
  return a.x + b.y; // 30
}
```

The `StructFieldInit` structure in `src/shape_lower.rs` is a **source-order-preserving pre-lowering candidate**, not yet a production typed AST node. Its concrete declaration is resolved from the bounded file-local struct declaration table. Only after all field/shape/position checks does the compiler synthesize `new Particle(10,20)` tokens. The ordinary `parser::parse_args` remains unchanged: `func(x=1)`, `obj.method(x=1)`, `new Class(x=1)` and `spawn Actor(x=1)` are NOT generalized named calls. Ordinary named call labels continue to be `x: value` where supported.

The pre-lowering now maintains an explicit **brace-scope and callable-header ledger** tied to the original source token indexes, so a local/formal named `Particle` inside a separate brace-bodied function does not shadow a `Particle(...)` initializer in `main`. Global callable/class/record/import/name conflicts remain source-wide. A same-scope or enclosing-scope local/formal still blocks the shorthand. The ledger is deliberately conservative for unrecognized/end-delimited callable syntax, imported symbol binding, complex nested scopes and ownership-sensitive expressions; those are not yet a full production typed resolver. Synthesized constructor tokens have no original source index and cannot impersonate an initializer candidate. No fallback is permitted from a failed ordinary call into a struct initializer.

## Typed-IR admission follow-up

`StructFieldInit` still exists in the dedicated source-local lowering pass rather than the general parser AST. The next compiler integration must bind a verified nominal declaration symbol to ordered field expressions in typed IR, evaluate expressions to temporaries in source order, then permute only the temporaries into declaration-order constructor slots. It must preserve partial-initialization cleanup, borrow/move semantics and error effects. This PR intentionally does not add a generic `NamedArg` with equals labels.

## Limitations and safety boundary

- Runtime-computed fields, calls, negative/non-scalar values, nested aggregates, generic structs, defaulted/optional fields, inferred anonymous structs, module-qualified constructors, custom constructors, class/record/actor construction, and arbitrary ownership-sensitive fields **remain unsupported**. This subset does not relax existing `infer struct{...}` semantics.
- Reordered literal fields are safe: literal tokens have no side effects. The current guest constructor emits fields in declaration order. For `Particle(y=read_y(), x=read_x())`, evaluating source expressions in declaration order would be wrong. This branch deliberately returns `E_UNSUPPORTED` before guest codegen; ordered typed temporaries plus unwind/ownership semantics are required to extend the subset.
- No `name=expr` to `name:expr` lexical transformation is permitted. Generic bindings `Array<T=Particle,layout=soa>`, assignment `=`, and value equality `eq` remain separate grammar categories. Avoid formatting/source-reordering until a parser-aware semantic equivalence check exists.
- AoS/SoA/AoSoA allocation and `Array<Particle,layout=soa>` are separate unfinished tasks in [compiler PR #312](https://github.com/atto-lang/atto-source.rs/pull/312) and [spec PR #77](https://github.com/ores-truffle-oreslang/attolang-language-spec.md/pull/77).

## Verification gates

```sh
cargo check --locked --all-targets
cargo test --locked --test struct_field_equals --test preview_data_shapes
cargo build --locked --release --bin attoc
ATTOC="$PWD/target/release/attoc" node tests/struct_equals_guest.mjs
```

The Rust integration tests invoke the compiler and check emitted Wasm magic and no-artifact errors. The Node test independently compiles the same fixtures through the native compiler, **instantiates the real generated guest Wasm and checks its integer return values**, checks determinism and negative compiler exit status without output artifacts. It fails when `ATTOC` is missing instead of skipping.

No one may claim these checks passed merely because they are committed. Require exact source SHA/tree, toolchain versions, actual positive job steps, compiler binary provenance, stdout/output/exit receipts and independent backend evidence. Source Actions with zero steps are NOT_EXECUTED. Private cross-org CI mirroring is disallowed without separate approved export authorization.
