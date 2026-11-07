# AttoLang portable browser compiler (experimental M1 candidate)

This repository contains a Rust scalar compiler capable of native execution and compiling itself for browser WebAssembly. Its strict AttoLang subset uses a typed i32 stack IR and a bounded direct core-Wasm generator. The implementation is **not** the full canonical AttoLang compiler or language parity.

### Build

```sh
rustup toolchain install 1.85.1 --profile minimal --target wasm32-unknown-unknown
cargo test --locked
cargo build --locked --release --target wasm32-unknown-unknown
node tests/wasm-browser.mjs
```

The compiled `atto_compiler.wasm` exposes the existing playground ABI. The guest it produces exports `main:()->i32`, with zero imports, no memory, no start section. Only one compiler worker owns an instance, and the caller copies results before compiling again.

The strict subset supports signed i32 literals, `fnc main()->int`, arithmetic + - * /, parentheses, integer bindings immutable by default and explicit `let`/`mut` rebinding. Max source 4096 UTF-8 bytes, 64 locals, 128 nesting depth, bounded code and module bytes. Unsupported constructs reject rather than silently fallback. The subset's i32 wrapping and division trap behaviors are experimental, not canonical AttoLang overflow policy.

Do not publish this compiler in the marketing playground until a version-pinned, exact-head hosted build, browser ABI, negative rejections, source-dependent guest execution, and independent review all pass. The original C M0 playground remains a separate verified artifact.
