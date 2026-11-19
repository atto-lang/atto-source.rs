# Exact public playground sample conformance

This directory pins **all nine** runnable `.atto` examples from
`atto-lang/atto-lang.github.io` at upstream site Git commit `c5ce2324d6277e91817ac2fb4b4e12f7b838afa7`.
`manifest.json` gives their exact Git blob SHA-1 and expected outputs,
diagnostic or panic category. The conformance harness recomputes each hash
before compiling; no moving upstream branch, mock evaluator or unreviewed
binary is consulted.

`node tests/conformance.mjs` compiles these real sources with the
wasm32-compiled Rust compiler and executes the generated guest Wasm.
When `ATTOC` is supplied, it requires byte-identical output from the
native Rust compiler; the rejected example must emit **no artifact**.
The actor case additionally checks compiled mailbox admission/delivery
traces; the overflow case checks panic_code=1 and preserved output.

Example classification remains owned by the public site's
`play/example-backend-profile.json`. Its syntax previews must not
become runnable simply because a private compiler branch accepts them.

**Limit:** This corpus proves only the browser guest-v2 subset. The
actor example is intentionally a legacy direct-state compatibility
profile; enums use a design spelling from LLVM. It does not establish
equivalent Java/GraalVM, LLVM, Forge, .NET, independent Rust/Cranelift,
or LLVM-to-Wasm bridge behavior. Those require matched exact-source
executed guests, negative cases, and trace contracts per target.

No candidate compiler binary or source was copied into the public site,
and no public release/promotion is permitted by these tests.
