# AttoLang `std.process`: Rust compiler / wasm32 target

This repo owns its own `stdlib/process_policy.atto` and an import-based
`tests/fixtures/std-process-policy-v1/main.atto` regression fixture.
The module provides **pure admission arithmetic only** and does not expose
OS child-process capabilities. The fixture is intended to return 42 when a
compatible compiler import/typecheck/execution path exists.

## Target capability contract

- The default browser-oriented `wasm32-unknown-unknown` guest has **no
  host process spawn capability**. A call to `process.spawn` must be rejected,
  not lowered to `Worker`, `WebAssembly.instantiate`, or a synthetic PID.
- The Rust compiler can be run natively, but that does **not** mean its
  generated browser guest may launch a child OS process.
- Any future separately-authorized native/WASI host-process extension
  must implement `await process.spawn(Command)` as
  `Result<ChildProcess, ProcessError>` with an owned generation-bound
  handle; no implicit shell or inherited environment.
- Permission enforcement occurs at the actual host boundary, including
  RUN command scope, actor capability attenuation, cwd/env/file rights
  and per-owner atomic quotas. Pure .atto preflight is insufficient.
- Capture stdin/stdout/stderr as bounded async byte streams; observe exit
  without blocking an actor carrier. Handle cancellation, timeout, forced
  termination, exactly-once reaping, and cleanup of inherited descriptors.
- POSIX `fork` must not clone the multithreaded actor runtime; on
  applicable native targets prefer `posix_spawn`; Windows uses
  `CreateProcess`. Expose raw `fork` only as a separately reviewed,
  platform-restricted primitive, if at all.

**Evidence:** checked-in source and expected behavior, NOT EXECUTED on the
real Rust compiler/guest Wasm. Native process launch and AOT NOT VERIFIED.
This draft is not functional `std.process` runtime parity.
