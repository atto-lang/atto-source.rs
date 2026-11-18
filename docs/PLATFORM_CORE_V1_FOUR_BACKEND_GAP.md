# Java / LLVM / Wasm / Rust: platform-core-v1 scalar candidate gate

The `contracts/platform-core-v1/` candidate fixtures and manifest were copied
**within the same private `ores-truffle-oreslang` organization** from the Java
canonical corpus. Each committed Git blob is byte-identical to the corresponding
Java, LLVM and Wasm file (compare immutable Git blob SHA values independently).
No source bytes went to another GitHub organization or test carrier.

This is **not** proof of full backend parity. The Rust compiler is a separate,
bounded Cranelift source compiler, not a wrapper around Java/LLVM and not a
general platform runtime. The Wasm path currently delegates to a pinned native
LLVM compiler and admits a zero-import `() -> i64` guest; it does not provide
a full actor/task/FFI ABI.

## Native execution acceptance

Run on a supported Linux x86-64 host with Rust/C toolchains:

```sh
cargo fmt --all -- --check
cargo test --locked --test platform_core_v1 -- --nocapture
cargo test --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
```

`tests/platform_core_v1.rs` checks canonical manifest coverage, keeps
unsupported actor/state/ownership/async/stdio syntax fail-closed, compiles the
seven exact canonical positive files with checked i64 semantics, and then
compiles the **same expressions** under a renamed export so a real native C
observer can compare full-width signed-i64 stdout. The export rename means
those executed files are **transformed** source, and must not be misreported as
byte-identical original-source execution. Five negative source files must
reject; the overflow case must emit a checked native trap instead of wrapping.
Nothing in these tests certifies Java/LLVM execution, Wasm target execution,
standard-library imports, physical hardware, hard real-time, or guest actors.

Before any parity promotion, independently execute the **exact original** guest
fixtures on all four backends, including a trusted print/observer ABI or full
signed-i64 result channel; correlate exact source/tree SHA, compiler artifact
digest, test steps/logs, diagnostics and normalized outputs. Do not treat
source readback, unit schema tests, generated CI job objects, or skipped suites
as executed compiler tests.

## Remaining major gaps

- Rust: actor creation, Mailman/supervision, sealed actor state, borrow/share/take
  and pinned async loans, throw/raise/panic, Task/select/channel, package/module
  imports, `ores.std/testing`, C/JNI/OS bindings and platform-specific typed
  compiler/runtime lowering are still incomplete. Fail closed.
- Wasm: cannot equate an LLVM-to-Wasm scalar artifact with a general portable
  guest actor ABI, protected host integrations, worker supervision, platform
  device I/O or deterministic hard real-time guarantees.
- Java vs LLVM: compare actual guest executions and negative diagnostics;
  a matching fixture Git SHA does not prove lowering or runtime equivalence.
- Platform APIs (mobile handoff, IoT stream hints, desktop resize bounds,
  embedded IRQ depth, FPGA/ASIC evidence pipelines) require **separate**
  compiler-exposed APIs, ownership/FFI guards, lifecycle checks, native/device
  target profiles and integration tests. None is established by scalar tests.

## Security/release block

Current source-org Actions often fail before any runner executes. Private
source transfer to another GitHub organization is **on hold** pending
conformance P0 incidents #22 and #77. Do not repeat cross-org mirroring to
obtain CI minutes. Use source-local authorized runners, independent approval,
effective private `main` protections, source-commit and full-tree pins, and
execution logs before marking this work ready or merging.
