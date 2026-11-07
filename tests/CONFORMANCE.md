# Canonical scalar conformance v1

These fixtures were copied from \`ores-truffle-oreslang/oreslang-source.java\` (\`main\` commit \`2804333264e4bbfcc9015d02b63b3caf11a3ada1\`), sourced from \`contracts/v1/semantics/fixtures\`, plus \`contracts/v1/fixtures/nested-comments.ores\`. LLVM carries the same shared contract.

Implemented Rust fixture subset:
- **Native:** add, precedence, forward, underscores, unary, owned-identifier, nested-comments.
- **Diagnostic classes:** unknown_name, call_arity, duplicate_declaration, integer_overflow, unterminated_comment.
- **Pending:** general borrow/ownership violation, try/finally, defer, actors, Task continuations, async, non-scalar types, runtime ABI parity, overflow arithmetic semantics beyond the portable no-overflow subset.

The \`tests/conformance.rs\` tests require compilation of all positive inputs, and exact classification of negative inputs. The GitHub workflow executes the positive cases through a real host linker and program, checking exit codes. No JSON-contract checker alone is treated as native execution evidence.

## Native symbol visibility: Rust-only executable parity gate

Canonical new source: `tests/fixtures/private-linkage.atto`, using `pub fnc answer`
and a non-public `fnc bump` helper. `src/codegen.rs` now emits `Linkage::Export`
for public functions and the executable entry `main`, and `Linkage::Local` for
other private functions. This preserves internal calls while preventing a
private symbol from becoming a public C/native linker API.

The Linux `native_object_execution` integration test checks the emitted ELF
global symbol table, links and executes an external C caller of `answer` (exit
42), and requires external linking against `bump` to fail. A separate
`profile_cli` test repeats public/native execution and symbol isolation with
`--strict-aot`. An implicit non-`pub` `main` still links, matching the existing
entry-point convention and release tree-shaker root.

**Evidence boundary:** these are Rust/Cranelift compiler-backed native guest
tests, not Java/LLVM execution receipts. Until this PR's exact-head CI runs
and the actual linker tests pass, record them as **NOT_EXECUTED**. Actor
ownership, isolation, capability and runtime parity remain **NOT_CERTIFIED**.
