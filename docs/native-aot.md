# AttoLang native AOT compilation — initial import-free profile

**Status: draft; source implementation exists; end-to-end execution evidence is pending.**
This adds *native machine-code precompilation* to the Rust AttoLang
`atto-source.rs` compiler branch built on Array<int> slice PR #178.
It is not a declaration of complete language/backend parity.

## Three levels that must not be conflated

| Output | Built before guest execution? | Native machine code artifact? | Requires execution host? |
| --- | --- | --- | --- |
| `attoc program.atto -o program.wasm` | Yes: AttoLang source lowered to Wasm binary | No: portable Wasm code | Yes: a Wasm engine may JIT |
| `attoaot program.atto -o program.cwasm` | Yes: AttoLang + Wasm-to-machine compilation | **Yes**, target/Wasmtime-specific precompiled module | **Yes**, compatible Wasmtime loader; no compilation required on load |
| Standalone `program` ELF / Mach-O / PE | Not provided by this stage | Not yet | Needs new runtime-only embedding/linking target |

A `.cwasm` is **not** a native executable file. It contains precompiled
code compatible with its CPU, Wasmtime version/configuration and target host.
It cannot be assumed transferable across hosts.

## Build-time usage

Use a **trusted** Wasmtime 50.0.1 CLI, matched to the runtime host:

```sh
cargo build --locked --release --features native-aot --bin attoaot

./target/release/attoaot tests/fixtures/pass/294-array-contiguous-slice.atto \
  -o /tmp/atto-slice.cwasm

# Optional cross-target compilation for a known supported Wasmtime triple:
./target/release/attoaot tests/fixtures/pass/294-array-contiguous-slice.atto \
  -o /tmp/atto-slice-x86_64.cwasm --target x86_64-unknown-linux-gnu
```

`--wasmtime /path/to/wasmtime` overrides the build-time compiler location.
The packager never invokes a shell; it calls `wasmtime compile` with
argument boundaries and stages source Wasm in an isolated, transient directory.
It will **never overwrite** a final output file and it publishes only a
completed, non-empty `.cwasm` artifact. It fails if Atto compilation,
native code generation, or filesystem publication fails. No source, guest
Wasm, or native compiler needs to be present when executing a precompiled
artifact; the compatible loader/runtime is still required.

**Import-free pure guest only in this first AOT lane.** Our browser guest
ABI imports `atto.print_i64`, `atto.actor_spawn`, etc., rather than
WASI. The generic Wasmtime CLI cannot instantiate these without a linking
host. The packager parses the generated Wasm import section and rejects
nonzero imports with `E_AOT_HOST_IMPORTS`; do not silently accept a binary
that cannot run. A future full native AOT runtime host must implement the
exact import and actor/control contracts with no JIT/compiler dependency.

## Run a precompiled artifact

On the same supported platform with a matching trusted Wasmtime runtime:

```sh
# The AttoLang guest currently exports main() -> i64, not WASI _start.
wasmtime run --invoke main /tmp/atto-slice.cwasm
```

`--invoke` is an unstable Wasmtime CLI feature; runtime-only embedded
Wasmtime hosts should use the typed `main` export directly.

Native AOT deployment should use a dedicated Wasmtime runtime build with
`runtime` enabled and `cranelift`/`winch` disabled. That configuration
loads trusted precompiled artifacts without any compiler/JIT tier.
The stock Wasmtime CLI may itself contain a compiler, even if it executes
a precompiled module without calling it.

**Security:** `.cwasm` contains native code; only execute artifacts
from trusted build pipelines. It is not safe to deserialize arbitrary
untrusted serialized modules. Treat the artifact as a native binary and
record its Wasmtime version, target CPU, source SHA, and build options.

## Verification and CI admission

```sh
cargo test --locked --features native-aot --bin attoaot --test native_aot
ATTO_REQUIRE_NATIVE_AOT=1 cargo test --locked --features native-aot \
  --bin attoaot --test native_aot -- --nocapture
```

The second command requires a real Wasmtime tool on PATH; missing Wasmtime
must fail rather than falsely report AOT coverage. A successful gate
requires (1) source compilation (2) machine-code artifact creation (3)
source deletion (4) loading/instantiation of the precompiled module and
(5) an actual `Array<int>` slice guest result of 30. Tests additionally
cover imported guest refusal, unavailable native compiler and file-overwrite
protection.

The `native-aot` Cargo feature is **host-only**, preserving a dependency-
free browser-Wasm compiler build when the feature is disabled. Rust’s host
`attoaot` tool is compiled natively ahead of time itself; it is not a JIT.
Avoid Python scripts for the build/test path.

The documented CI lane uses pinned Wasmtime 50.0.1 and fails closed if
runner admission does not execute the AOT smoke job. Do not infer backend
parity from a Rust-host test or a job with zero execution steps.

## Deferred for full product-grade AOT

1. Import-bearing stdio, filesystem, network, actor and `suspends` operations
   require a native runtime host ABI, deterministic guest lifecycle and
   verified exception/control-path delivery.
2. Standalone native ELF/Mach-O/PE builds require linking a runtime-only
   Wasmtime embedding and a trusted `.cwasm` into a native launcher, or
   alternative direct LLVM/Cranelift/RS code generation. Remove JIT/Cranelift
   compiler features from the runtime loader and test the linked executable.
3. Cross-compile and run the target architecture/runtime matrix with exact
   source and artifact hashes, ownership/borrow checks, ASan/UBSan where
   applicable, invalid-import/invalid-bounds tests, and no source compiler
   present on the target device.
4. Integrate the array branch and syntax PR #174, complete non-unit strides
   and iterable spread, then replicate verified semantics on other AttoLang
   backends. Until then, unsupported constructs must fail closed.

Reference: [Wasmtime CLI AOT compilation](https://docs.wasmtime.dev/cli-options.html),
[precompiled modules](https://docs.wasmtime.dev/examples-pre-compiling-wasm.html),
and [minimal runtime embedding](https://docs.wasmtime.dev/examples-minimal.html).
