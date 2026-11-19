# Integrated Rust scalar parity, source-owned — 2026-11-07

Status: **NOT_EXECUTED / NOT_CERTIFIED**; do not merge or release based on this document or a conflict-only mergeable state.

Base main `e516b09622f6d1b9d61330aff88e6c86bb6d97f1` tree `1768fc4599596ea09d16cb39620b1c8d345f91f2`. Pinned source PR heads:
- #25 `6260ef9311adda5f352d6138f3b534667f77fac2`: source-owned exact checkout gate and hostile worktree regression tests
- #28 `29398535fe99679cfc85f948b9c50f6b70c36c5c`: scalar immutability and canonical `.atto` fixture
- #29 `4c7fcbbe19aee66a719678260b9cad9f248d3b0f`: line/comment scanner and full-width signed-i64 guest fixtures
- #30 `8cebadc6e8c609efc2bc826580d0ba649827fd7e`: private symbol isolation and native ABI probes
- #31 `0de639870b245bfc8b9dbe9706f83c9629e70ab6`: 17 LLVM-sourced portable scalar guest programs

All unconflicted files are included by their **unaltered Git blob IDs**. The only semantic merges are `src/frontend.rs`, `tests/native_object_execution.rs`, and `tests/CONFORMANCE.md`. The source export authorization remains disabled; no external carrier is approved by this integration.

Required independent exact-head gates:
1. Verify expected source commit SHA, exact tree, index/worktree cleanliness, all source blob provenance, and no external source export.
2. Run source-owned `verification/ci-worktree-v1/verify.sh` with `EXPECTED_HEAD`, then `verification/ci-worktree-v1/test.sh`, and re-verify original head.
3. Run `cargo fmt --all -- --check`; `cargo test --locked --all-targets`; `cargo clippy --locked --all-targets -- -D warnings`.
4. Require **executed** Cranelift ELF guest tests, 17 LLVM scalar source fixtures, packed ticket/receipt, full-width signed-i64 C oracle, private linker visibility, and normalized semantic diagnostics. Obtain independently verified Java and LLVM guest receipts.
5. Obtain independent PR review and enforce required main protection before merging. A workflow with zero executed steps is missing evidence, not a passing test.

No claim of checked-overflow, actors, per-actor heaps/stacks, ownership, async/select or hard-RT parity. Overflow/division are out of scope of the shared portable-i64 contract.

## Follow-up reconciliation: PR #32's missing immutable binding surface

Identified the parallel draft [#32](https://github.com/ores-truffle-oreslang/oreslang-source.rs/pull/32) at exact head `8a8e118e69e8c5958c4ad5ec6b084d2564c2dc29`. Its additional `val name: int` parser branch is now included without losing the #29 line-comment scanner, #28 immutable diagnostics, or #25 physical exact-source gate. Reused its existing `tests/immutable_binding_parity.rs`, `tests/fixtures/immutable-bindings.atto`, and `docs/RS_IMMUTABLE_BINDING_PARITY.md` Git blob IDs unchanged. Also included its native `.atto` CI compile+link+execute step and conformance documentation.

This incremental merge is **source review evidence only**; tests remain NOT_EXECUTED and no private source export or CI runner authorization is implied. PR #32 should not be independently merged ahead of the reconciled branch.
