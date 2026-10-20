# Canonical scalar conformance v1

These fixtures were copied from \`ores-truffle-oreslang/oreslang-source.java\` (\`main\` commit \`2804333264e4bbfcc9015d02b63b3caf11a3ada1\`), sourced from \`contracts/v1/semantics/fixtures\`, plus \`contracts/v1/fixtures/nested-comments.ores\`. LLVM carries the same shared contract.

Implemented Rust fixture subset:
- **Native:** add, precedence, forward, underscores, unary, owned-identifier, nested-comments.
- **Diagnostic classes:** unknown_name, call_arity, duplicate_declaration, integer_overflow, unterminated_comment.
- **Pending:** general borrow/ownership violation, try/finally, defer, actors, Task continuations, async, non-scalar types, runtime ABI parity, overflow arithmetic semantics beyond the portable no-overflow subset.

The \`tests/conformance.rs\` tests require compilation of all positive inputs, and exact classification of negative inputs. The GitHub workflow executes the positive cases through a real host linker and program, checking exit codes. No JSON-contract checker alone is treated as native execution evidence.
