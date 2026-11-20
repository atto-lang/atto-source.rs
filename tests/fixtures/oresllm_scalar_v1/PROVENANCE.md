# Exact-source scalar parity corpus

Source: `ores-truffle-oreslang/oreslang-source.llvm` at commit `15bdd5cc48cf64387aee7e19f2926c253eb10e08`.
Each `.atto` fixture below preserves the exact bytes of the corresponding legacy
`.ores` file under `contracts/v1/semantics/fixtures/` in the LLVM repository.
The Git blob SHA-1 is therefore a reproducible byte-for-byte provenance check.
All 17 of these cases have expected native exit status **42** in LLVM
`contracts/v1/semantics/cases.json`. They test only portable i64 arithmetic
whose intermediate results are representable in signed i64.

| Case | Git blob SHA-1 (LLVM original and Rust copy) |
|---|---|
| oresllm-precedence | `116c261c259f5d8753c3c827595243403f22045f` |
| oresllm-scalar-gradient | `fbd4877db8dc2edf8a02fb76ee738d24cb69f922` |
| oresllm-signed-product | `901337ede5e2a7652837ed3f513073c63edb887e` |
| oresllm-grouped-negative | `22e8926b292cc53effe40fe506c0216b89677af9` |
| oresllm-nested-precedence | `f3fda2718ea88e734b30df4d6c1012191cc3b77a` |
| oresllm-max-minus-one | `cc3d33b3f9c8643fe335001d1c6f6a777276d314` |
| oresllm-zero-product | `233b6e5f21ad016ea1c2e206a753f48340217824` |
| oresllm-nested-arithmetic | `cae0cd83d51f65d137aa500aa7b7040313e3de66` |
| oresllm-subtract-self | `d1b46d5ea8bedf5af1954bdfdff84d30ffdd7d6b` |
| oresllm-zero-add | `08b5752db0e4b40d81d0e3ba247ff11d0594be67` |
| oresllm-signed-subtract | `7797ff45062a684f0af0533beed3952d4899564b` |
| oresllm-left-associative | `0a65516453f36a8e26fd53a0ec7b179821717ceb` |
| oresllm-zero-times-max | `f0b1eafaefc2b58d64d2c70bc42accdf37e6c715` |
| oresllm-over-2pow53 | `3fef943aa0adef685f888fcaa1506adbc03701f6` |
| oresllm-min-max-cancel | `452a4b56d1a13fe8be82b48f31a95cd564de1eab` |
| oresllm-near-square | `f49e393c7b0d6e2b324f8315294964936de9f265` |
| oresllm-over53-cancel | `919c6d0aee262ea506388c9ef636330334cb568c` |

**Evidence status:** Source-copy and test definitions only. Rust native execution
is **NOT_EXECUTED** until exact-head Linux CI compiles, links, and runs every
guest. Java and LLVM execution is **NOT_CERTIFIED** by these Rust tests.
Overflow arithmetic behavior, actors, borrow checking, and task/runtime parity
remain outside this corpus. No private-source CI mirror or external runner
is authorized by this change.
