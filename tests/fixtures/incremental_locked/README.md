# Locked-package incremental fixture

12 actual Atto sources: ten files declaring three explicit namespaces, two files without namespace headers, eleven explicitly imported distinct modules. Baseline guest returns 42; one body edit in unnamespaced lib/m10.atto changes the result to 43. Immutable lock inventory is constructed in the test. The current Rust profile performs whole-package graph admission and final artifact reuse only; no namespace-level semantic reuse claim. A second importing package and shared per-unit graph counters are follow-on gates (tracker #53).
