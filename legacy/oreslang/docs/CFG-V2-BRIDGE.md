# Rust compiler → analyzer CFG-v2 bridge

\`oresc --cfg-ir program.ores 16\` exports a versioned CFG-v2 JSON envelope **from the currently admitted scalar-only Oreslang MIR**. For now the compiler generates one explicit entry/return block per function. It preserves variable definitions, reads, compute-cost nodes, and direct calls.

The independent Rust analyzer can consume it with:

\`\`\`sh
cargo run --locked --bin oresc -- --cfg-ir examples/answer.ores 16 > /tmp/ores-cfg.json
# From an analyzer checkout:
cargo run --locked -p ores-analyze -- --cfg /tmp/ores-cfg.json hard
\`\`\`

Do not claim this is full Oreslang CFG lowering. It is a conformance interface and incremental bootstrap toward typed CFG/SSA, control-flow joins, verified direct-call summaries, source provenance, actor/Task borrow lifetimes, exception paths and target-specific real-time verification.

A direct call is emitted as an explicit \`call\` event and is **rejected** by the current analyzer rather than implicitly approved. Omitting a cooperation budget is rejected in RT profiles. The compiler does not synthesize or certify WCET.

Existing Java/GraalVM and C++/LLVM semantic fixtures remain the normative scalar source corpus; expand feature gates only after the same programs compile and execute natively on all three backends.
