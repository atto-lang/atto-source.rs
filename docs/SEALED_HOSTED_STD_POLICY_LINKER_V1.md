# Compiler-owned hosted std pure-policy imports — compiler milestone

**Status: implementation authored; native compiler/guest CI NOT_EXECUTED.**

This branch extends the existing **real** bounded relative-file linker in
[src/import_link.rs](../src/import_link.rs) to support two *allowlisted, compiler-embedded*
built-in module identities using the canonical package-free Atto syntax:

    import * as http from "std.http";
    import * as fs from "std.fs";

and selective imports such as:

    import http_status_success as is_ok from "std.http";

These identities currently expose **only the pure-policy helpers** in
[stdlib/std/http_response_policy.atto](../stdlib/std/http_response_policy.atto)
and [stdlib/std/fs_text_policy.atto](../stdlib/std/fs_text_policy.atto),
[stdlib/std/fs_api_policy.atto](../stdlib/std/fs_api_policy.atto), and
[stdlib/std/fs_io_policy.atto](../stdlib/std/fs_io_policy.atto).
The compiler embeds a 19-function pure filesystem policy bundle (3 text,
11 API admission and 5 byte-I/O functions); this is still **not** an FS runtime.
The sources are imported directly using Rust include_str! into the compiler,
without a network fetch, mutable filesystem read or fabricated/translated
host-language implementation.

- Canonical HTTP policy blob: 22757dafe89bc9601d04c8726a09ddf676206c7e
- Canonical FS text policy blob: 0cd53216e91ef8de1a764ba5b80ce85c5796c1f2

Both are Git *blob* IDs; do not confuse source byte equality with executed guest
parity. These exact files are on Java, LLVM, .NET, Forge, Cranelift, Wasm,
Rust, and JavaScript candidate PR branches, and on atto-common-source #29.

## Compiler paths

- **Single-file browser compiler compile(source):** if source begins with an
  import declaration, it can resolve the two allowlisted sealed policy
  built-ins in-memory; ordinary relative imports still fail without a trusted
  source loader.
- **Host compile_with_imports(source,path,load):** uses the same trusted
  source-graph lowering and never calls load for the two sealed identities.
  Real local relative imports continue to use the explicit loader.
- **Native attoc INPUT.atto:** invokes compile_with_imports through its
  existing bounded project loader; its built-ins are independent of
  files with matching paths in the user project.
- **Native attoc --package-root:** uses a separate graph linker
  (package_resolver/linker.rs), and is **not** included in this first sealed
  std module profile.
- Namespace-first `use namespace storefront;` and logical-path-derived
  `use default namespace;` are admitted for this **restricted pure-policy
  function import profile**, with the canonical namespace declaration
  validator. This is not general package namespace export/re-export support:
  the separate `--package-root` linker still requires its own integration.

This reuses the existing typed module flattening and preserves its namespace
shadowing, private member, duplicate binding, malformed source and no-artifact
failure behavior. The import cache distinguishes sealed package IDs from any
local source file that happens to have the same relative path.

## Tests / exact required evidence

```sh
cargo test --locked --test hosted_std_policy_imports
cargo build --locked --release --bin attoc
node tests/hosted_io_pure_policy_guest.mjs
```

The Rust tests require the ordinary guest compiler to produce Wasm for the
two sealed policy imports and reject unknown pkg imports, forged symbols,
missing members such as http.get/fs.read_text, and local path masquerading.
The Node acceptance test compiles the **actual import-bearing .atto source**
using the Rust attoc executable, validates and instantiates the emitted
WebAssembly with **zero host imports**, requires main() == 42n and checks
additional failure cases emit no Wasm.

This is not a string-level source concatenation test. Only a nonempty
exact-commit workflow or equivalent trusted executed run can establish PASS;
the branch has NOT_EXECUTED until that receipt exists. Local Node host I/O
tests do not change this status.

## Outside this milestone

The pure-policy bundle does not implement std.http.get(), HttpResponse.text(),
std.fs.write_text(), std.fs.read_text(), async suspension/cancellation,
DNS/TLS, actual disk roots, typed guest I/O results, browser permission
brokering, or cross-backend guest execution. Calls to these unsupported
members must **fail closed**, without emitting an executable artifact.

Next integrate the existing Rust async #186 and HTTP wire #386 with FS
admission #412, the host-only Wasm I/O adapter #43, and common stdlib
#29 after language/security review. The true HTTP->FS->readback conformance
fixture is tracked under atto-platform-conformance #101 and remains
NOT_EXECUTED on every target.

## Canonical import migration and expanded source checks

The preferred import is `import * as fs from "std.fs";`, not
`from pkg "std.fs"`. The exact two old `from pkg` identities remain a
**temporary compatibility alias** for the SHA-pinned original 56-vector
fixture; they still resolve exclusively to compiler-owned embedded sources,
never to an external package. Unknown `core.*`, `alloc.*`, or `std.*`
modules must return `E_STDLIB_UNLINKED` without invoking the guest file
loader. Unrecognized third-party `pkg` dependencies remain unsupported.

The new `hosted_std_policy_imports.rs` test stages actual source-to-Wasm
checks for `fs_open_options_valid`, `fs_io_next_chunk`, and the hardened
create-new copy destination predicate. The Node guest test likewise compiles
their actual Atto source with zero imports and expected exit 42, while
requiring unknown modules and unimplemented `read_file`/HTTP I/O to fail
closed with no executable artifact. These are **authored checks**, not a
current-head successful compiled guest receipt. A link to real filesystem
I/O is still prohibited until a versioned actor/tenant-checked host ABI,
permissions and async continuation lowering are proven.
