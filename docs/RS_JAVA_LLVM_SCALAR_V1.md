# Rust/Cranelift vs Java/LLVM scalar-v1 — security audit (2026-11-08)

**DRAFT / NOT_EXECUTED / NOT_CERTIFIED.** This is source-level implementation and
test definition only; an exact-head runner with real nonzero job steps is still
required. An unadmitted CI job is not a compiler failure or a pass.

## Audit findings addressed

1. **Noncanonical removed-operator diagnostics.** Prior Rust returned generic
   `syntax_error` on %, ^, and ^^. The lexer now recognizes all three only
   to reject them immediately as `removed_operator` with an exact source byte
   offset, matching the canonical diagnostic family after dash/underscore
   normalization. No executable/IR/operator behavior is introduced.
2. **Recursive unary source could evade the 64-parenthesis cap.** Nested unary
   minus invokes the parser recursively without parentheses. The parser now
   enforces a 64-level primary recursion budget and fails closed with
   `resource_limit` before recursing deeper.
3. **Long additive/multiplicative left folds could manufacture deep ASTs.**
   Enforce 64 operators per direct additive or multiplicative chain.
   This is an interim bounded scalar admission rule, **not** an arbitrary-depth
   parser or a complete semantic complexity proof.
4. **No persistent cross-backend fixture-identity gate.** Add 15 Git blob
   SHA-1 golden checks, and physical compiler-backed positive/negative paths.
   The fixture bytes are unchanged; this prevents subsequent accidental drift.
5. **Native ABI under-tested.** Extend with a four-argument, mixed typed
   parameter C caller using integer values larger than 2^53; verify
   signed 64-bit argument precision, order, and return value.
6. **Negative diagnostic coverage insufficient.** Add rejected `any`,
   `unknown`, `bool`, `float`, `Option<int>`, optional parameters,
   default params, duplicates, and call-arity cases. None of these types or
   features is implicitly enabled.

## Evidence requirements

On this exact PR head: `cargo fmt --all -- --check`,
`cargo clippy --locked --all-targets -- -D warnings`,
`cargo test --locked --all-targets`, source SHA/tree read-back, real
strict-AOT ELF/`cc` guest execution, sentinel nonpublication negatives, and
independent Java/LLVM guest receipts. Require independent review.

Rust currently does **not** implement canonical checked-overflow compile-time
rejection for the four remaining shared scalar-v1 negative files. Runtime
`--checked-i64` trap mode is not equivalent. Actor, ownership, threads,
32-bit actor-heap addressing, suspension and generics are outside the
implemented scalar subset; none is certified here.
