# Opt-in checked signed-i64 AOT parity pilot

Source reference: `ores-truffle-oreslang/oreslang-source.llvm` commit
`15bdd5cc48cf64387aee7e19f2926c253eb10e08`,
`tests/rust-harness/src/checked_i64.rs` (blob
`b9a82ef6411b2933e9f65a1b3ba7a4c9b5827b6e`).

**This is a semantic adaptation, not byte-identical source mirroring.**
The Java/Graal and LLVM full-runtime contract is not certified.

Compiler scope: `oresc --checked-i64 [--strict-aot] source.atto out.o`
or the Rust `compile_checked_i64(source)` API. The existing signed-i64
frontend, MIR and Cranelift object path are reused; arithmetic instructions
emit signed overflow flags and a conditional `INTEGER_OVERFLOW` trap.
Add, subtract, multiply and negate are checked, including through parameters
and calls. Signed minimum literal negation may be rejected at parse time.

The ordinary compiler remains unchanged (legacy wrapping native arithmetic).
Checked mode is deliberately **object-only**: the existing effect IR v1,
scalar-flow v1, safety-IR v1, CFG v2 and static facts v0 exporters do not
encode conditional trap edges. Combining `--checked-i64` with those export
modes fails closed; no falsely complete certificate is emitted. This also
means no claims about fully verified ownership, actor exception semantics,
WCET, hard-RT or complete backend parity.

The Linux x86-64 Rust integration test compiles each `.atto` guest through
the actual Rust compiler to a native ELF object, links it with `cc`, then
executes it. Positive cases must exit at the expected value; overflow cases
must terminate with SIGILL (signal 4), never return a wrapped result. Tests
cover direct and interprocedural overflow. Additional Linux architectures
need target-specific trap-signal conformance.

**Evidence**: SOURCE COMMITTED; execution NOT_EXECUTED until exact-head
source-owned CI runs with nonzero step logs. No private source export or
CI carrier approval was created by this PR.
