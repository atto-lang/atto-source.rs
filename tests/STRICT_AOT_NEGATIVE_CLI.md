# Strict AOT real-compiler .atto negative fixtures — Rust/Cranelift

**Proposed compiler-backed checks, not yet executed.** This PR adds 18
independent canonical `.atto` negative fixtures and 21 Rust integration
tests using the actual built `oresc` binary (`CARGO_BIN_EXE_oresc`).
They extend the existing scalar subset only; they do **not** certify
AttoLang actors, ownership, Proxy, garbage collection, or other backends.

Each fixture is matched against the bytes embedded in the test binary,
then validated with the real Rust compiler's `frontend::parse` or
`mir::lower` and the exact diagnostic class. The harness runs
`oresc --strict-aot` across --check, native-object, paired object/effects,
and the --effects, --static-facts, --cfg-ir and --safety-ir exporters.
All negative paths must exit 1 with the same diagnostic, emit no stdout,
publish no new object/effect files, and preserve preexisting sentinel files.

A positive .atto scalar control must pass --check and compile an actual
nonempty native object plus parseable effects JSON. **Native guest execution
is not asserted** in this PR.

The `eval` fixture currently rejects as an unknown function in this narrow
compiler, **not** a verified general strict-AOT dynamism guard. CPU/GPU
blocks are rejected with `execution_domain_unsupported`; %/^/^^ are
syntax rejections. This corpus must not be used to claim the 16 TARGET
negative classes from [spec #18](https://github.com/ores-truffle-oreslang/attolang-language-spec.md/pull/18)
are implemented across backends.

Existing `.github/workflows/ci.yml` already checks out the exact head
and runs `cargo test --locked --all-targets`. Once GitHub private-source
runner admission is restored, the following gives an independent executed
compiler-backed result:

```sh
cargo test --locked --test strict_aot_negative_cli -- --nocapture
cargo test --locked --all-targets
```

Until those commands actually run, status is **NOT_EXECUTED**. There is
no CI/security bypass, source transfer or certificate in this PR. Related:
[conformance model #587](https://github.com/ores-truffle-oreslang/oreslang-platform-conformance/pull/587),
[source CI admission #315](https://github.com/ores-truffle-oreslang/oreslang-platform-conformance/issues/315).
