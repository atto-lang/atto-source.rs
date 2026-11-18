# Explicit `borrow!` vs inferred call loans: executable regression matrix

**Status:** compiler-backed tests on top of draft [atto-source.rs #336](https://github.com/atto-lang/atto-source.rs/pull/336) (stacked PR). **Not** a claim that bare `Foo` function parameters borrow by default.

All `pass` fixtures must compile to guest Wasm and execute with their `expect-exit` value in both native and Wasm-compiled compiler paths. `fail` fixtures must reject with their `expect-error` code and produce no guest artifact. The repo's `tests/fixtures.rs` native suite and `tests/conformance.mjs` guest harness discover these source files automatically.

| Situation | Coverage |
|---|---|
| Explicit `borrow! p` and implicit `p` for `Borrow<T>` | `explicit-implicit-read-call` |
| Mixed explicit and implicit read loans of one owner across arguments | `mixed-read-arguments` |
| Named lexical readonly view, followed by exclusive mutable loan | `named-read-view-then-writer` |
| Two simultaneous named readers | `two-named-read-views` |
| Explicit call loan followed by `take!` | `explicit-read-then-take` |
| Distinct owners and independent loans | `separate-owners-explicit-read` |
| Exclusive mutable scope ends before read-call borrowing | `read-after-writer-block` |
| Readonly mutation, conflicting reader/writer, owned-formal confusion | negative fixtures |
| Computed temporary, subobject projection, escaped loan/reborrow, use-after-take | negative fixtures |
| Historical bare `Foo` parameter move is **still** observable | `legacy-bare-formal-still-moves` |

The following **language targets are not executable in this compiler slice** and must remain fail-closed until dedicated versioned implementations and tests: `fnc inspect(Foo p)` as implicit borrow (currently owned), `fnc update(mut Foo p)` plus `update(mut p)` as a mutable *loan* (not the same as a mutable owned parameter), `Borrow<T, lt=a>` lifetime return propagation, field-projection borrows, actor/worker transport and `borrow!` surviving `await`/select. This test PR deliberately avoids inventing success fixtures for these.

The direct expression `borrow! x` is **not** a universal ownership operator in any untyped context: outside a supported `Borrow<T>` call and named scoped binding, it must reject when the borrow would escape or be treated as an owned value.

To execute **only these 17 cases**, build the native and wasm32 compiler artifacts and run `ATTOC=target/release/attoc node tests/borrow_clarity_guest.mjs`. This focused runner verifies guest exits/stdout, exact native-vs-wasm compiler output bytes, and zero guest bytes with expected diagnostics for invalid sources. The full `node tests/conformance.mjs` suite also collects these fixtures, but its unrelated long-expression stress failures must be tracked separately and never hidden. Do not mark a source-only registry as proof of compiled execution.
