# AttoLang rs compiler agent directives

These instructions apply to human and AI contributors to this backend. Prefer canonical `.atto` guest source (legacy `.ores` only for compatibility); keep host/compiler/worker/ABI code minimal and capability-checked. Preserve Rust-only first-party tooling, no Python, independent security review and exact-head conformance requirements.

## Controlled dynamism and memory

Follow [organization directive PR #38](https://github.com/ores-truffle-oreslang/.github/pull/38), [language-spec PR #18](https://github.com/ores-truffle-oreslang/attolang-language-spec.md/pull/18), and the [backend checklist](docs/AOT_ZERO_GC_DYNAMISM.md). Strict **sealed full guest AOT** rejects runtime eval/source compilation, class/method injection, monkey-patching, arbitrary host reflection and hidden interpreter/JIT fallback *before executable code emission*. Keep dynamic dispatch, generics/macros, proxies and closure/async functions only through compile-time known targets, generated descriptors and statically lowered code.

All guest allocations, closure captures, task frames and actor heaps need explicit owner/region, `rt take`/`rt copy`/`rt borrow`/`rt share` checking, bounded admission and exactly-once release across cancellation/restart. Strong cycles require a documented strategy; long-lived actors must reclaim retired memory without waiting for termination. Function/closure transport by channel/pub-sub is prohibited. Maintain no implicit conversions/operator overloading, no `%`/`^`/`^^` and no automatic nested `Future`/`Option` flattening.

**Evidence:** declare host/guest AOT and guest/process GC scope; require compiler-backed positive and negative `.atto` fixtures, artifact provenance and stepful execution. Host-only, parser-only or `steps=[]`/`steps=null` means guest semantics **NOT_EXECUTED**. Existing experimental behavior is not certified.
