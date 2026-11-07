# Byte-identical Java Atto scalar checked-overflow negatives

Reference repository: `ores-truffle-oreslang/oreslang-source.java`
Reference commit: `c39100b400c4dc1475dd25f5946b136b7032d5f6`
Reference registry: `contracts/atto-scalar-v1/cases.json`
Reference registry evidence status: `NOT_EXECUTED`

| Rust copied fixture | Canonical Java Git blob SHA-1 | Normalized diagnostic family |
|---|---|---|
| `checked-add-overflow.atto` | `eed3113842125b1d2a295a11e3cc01a5c1e3da00` | `checked_overflow` |
| `checked-sub-overflow.atto` | `3f80e2eff35147984b040eb0a0215b439d20d09b` | `checked_overflow` |
| `checked-mul-overflow.atto` | `e608942c7e84fddb6ad7ca119a4a11d88aa9434b` | `checked_overflow` |
| `eager-unused-overflow.atto` | `ff0c520f2bc2efcc3daf032eb61fddc8f0c91d48` | `checked_overflow` |

Blob identity proves the source copies, **not** native guest execution, Java/LLVM runtime overflow behavior or complete checked-arithmetic parity. Runtime-dependent arithmetic remains unchecked in this Rust backend.
