# Rust compiler to data-layout analyzer bridge

The independent Rust compiler emits the same data-layout advisory IR v1 consumed
by `oreslang-static-analysis.rs`.

Use:

    cargo run --bin oresc -- --layout-ir path/to/source.ores

The output is JSON with `schema_version: 1`, `evidence_complete`, target
cache/vector metadata, and a list of layout regions.

## Proof boundary

The exporter consumes validated Oreslang MIR. It does not scan source text for
patterns and does not infer hotness, ownership, relocation legality, stable
addresses, or ECS suitability.

The currently admitted Rust frontend is scalar-only. It has no aggregate
storage, so its normal MIR contains no layout-bearing regions. For that subset,
an empty region list is complete and is pinned by
`tests/fixtures/scalar-layout-ir-v1.json`.

MIR carries an explicit `layout_evidence_complete` bit. The exporter refuses
to emit a manifest when that bit is false. When aggregate lowering is added, the
lowering pipeline must populate every relevant `layout_regions` entry before
marking evidence complete.

Each typed MIR layout region carries the analyzer contract directly: scope,
physical layout, row bound, declared field widths, accessed field indexes,
dense/sequential iteration evidence, hotness, relocation verification,
stable-address/whole-record constraints, iteration independence, sparse
membership, independently selectable component passes, and concurrent mutable
sharing.

## Security and resource limits

The producer independently validates the v1 evidence before serialization:

- at most 10000 regions,
- at most 16384 fields/access indexes per region,
- at most 1000000 field/access evidence items in total,
- region names at most 1024 UTF-8 bytes,
- field widths must be non-zero,
- accessed field indexes must be unique and in range,
- non-empty hot passes must identify accessed fields,
- target cache/vector widths must be null or powers of two >= 8 bytes.

These limits are mirrored by the analyzer so a forged public `mir::Module`
cannot be used to manufacture unchecked layout advice.

## Non-claims

Layout IR is an optimization evidence transport. It is not a proof of speedup,
cache misses, SIMD legality, memory safety, actor isolation, or WCET. SoA,
AoSoA, and ECS findings remain advisory until benchmarked on the actual target.
