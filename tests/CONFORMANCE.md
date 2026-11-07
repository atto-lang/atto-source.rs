# Canonical scalar conformance v1

These fixtures were copied from \`ores-truffle-oreslang/oreslang-source.java\` (\`main\` commit \`2804333264e4bbfcc9015d02b63b3caf11a3ada1\`), sourced from \`contracts/v1/semantics/fixtures\`, plus \`contracts/v1/fixtures/nested-comments.ores\`. LLVM carries the same shared contract.

Implemented Rust fixture subset:
- **Native:** add, precedence, forward, underscores, unary, owned-identifier, nested-comments.
- **Diagnostic classes:** unknown_name, call_arity, duplicate_declaration, integer_overflow, unterminated_comment.
- **Pending:** general borrow/ownership violation, try/finally, defer, actors, Task continuations, async, non-scalar types, runtime ABI parity, overflow arithmetic semantics beyond the portable no-overflow subset.

The \`tests/conformance.rs\` tests require compilation of all positive inputs, and exact classification of negative inputs. The GitHub workflow executes the positive cases through a real host linker and program, checking exit codes. No JSON-contract checker alone is treated as native execution evidence.

## Rust native i64 parity extension (November 2026)

`tests/source_i64_parity.rs` covers the seven expressions in the Java/LLVM
`contracts/v1/source-i64/portable-i64.v1.json` (shared Git blob
`bbc319c19ecd007109d33592e210091eb267e5a2`), plus two extra exactness
cases around 2^53. Each case compiles real Oreslang source to a Cranelift
object, links with a C oracle, *executes* the native guest, and compares the
full signed `int64_t` result. The C driver avoids the 8-bit exit-code
truncation of earlier scalar tests.

`tests/fixtures/proxy-packed-receipt.ores` is byte-identical to the existing
Java/LLVM shared guest fixture (Git blob
`13925491ac224bc5108097e73927968120eb182d`). The Linux integration
test compiles, links and executes the complete packed ticket/receipt program
and requires exit 42; this exercises exact `i64` arithmetic above 2^53.

The lexical regression tests ensure block delimiters inside `//` comments
do not alter source admission or the emitted effect contract; `//` ends at
LF or CR. A real unterminated block comment still fails with its exact source
offset.

**Evidence boundary:** these tests are executable acceptance gates, not proof
that a particular GitHub exact-head run has passed. Rust's checks do not
certify execution on Java/LLVM, source-level actor tickets, general ABI
parity, checked arithmetic overflow, division, or broader language/runtime
parity. The Java/LLVM portable-i64 contract is explicitly marked
`reference-only-not-compiled-execution`; backend execution receipts must
be independently verified before changing that status.

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
