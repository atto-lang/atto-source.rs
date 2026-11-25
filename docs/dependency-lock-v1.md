# Package management and reproducible builds v1 (`atto.lock`)

Status: **draft, first executable slice** (Rust compiler library).
Feature 6 of the 15 "additional features" proposals.

This builds on the existing single-package `.atto.pkg.lock` source pin
(`compile_package_locked`) and adds cross-package dependencies.

## Format (canonical form only)

```toml
# atto.lock v1: generated; do not edit by hand
[lock]
version = "1"
root = "acme@1.0.0"
offline = "true"

[[package]]
name = "base64"
version = "0.4.1"
source = "vendor:vendor/base64"
tree_sha256 = "<64 lowercase hex>"
deps = ""

[[package]]
name = "json"
version = "1.2.3"
source = "vendor:vendor/json"
tree_sha256 = "<64 lowercase hex>"
deps = "base64@0.4.1"
```

* `parse` accepts a file only if it is **byte-identical** to `render(parse(text))`.
  Packages are sorted by name, deps sorted, key order fixed, LF line endings.
  The same dependency set therefore always yields the same bytes, and there
  are no formatting-only diffs.
* `tree_sha256` is `tree_digest`: SHA-256 over a domain tag plus sorted,
  length-prefixed `(path, sha256(content))` pairs. Paths must be canonical
  relative POSIX paths (no `.`, `..`, absolute paths or duplicates).
* Strict grammar: unknown keys or sections, escapes, non-ASCII, CR and
  non-`MAJOR.MINOR.PATCH` versions are rejected.

## Semantics

| Check | Diagnostic |
| --- | --- |
| one version per package name (no silent duplicate resolution) | `E_LOCK_DUPLICATE` |
| every dep is locked | `E_LOCK_UNRESOLVED` |
| acyclic dependency graph | `E_LOCK_CYCLE` |
| vendored tree present / digest matches | `E_LOCK_MISSING` / `E_LOCK_HASH` |
| `offline = "true"` and a registry-only package | `E_LOCK_OFFLINE` |
| non-canonical bytes | `E_LOCK_CANONICAL` |

`install_order` is a deterministic dependencies-first order (Kahn's algorithm,
ties broken by name). `verify` returns the registry packages an online build
must fetch; each fetched tree must then match `tree_digest`.

**Trust:** as with `.atto.pkg.lock`, an unsigned lock next to mutable sources
gives integrity, not authentication. The lock's own digest must come from a
trusted channel (CI secret, signed release, reviewed commit).

## Tests

`tests/dependency_lock.rs` (10 cases): byte-stable round trip, canonical
rejection, tree-digest order independence, content and name sensitivity,
path escapes, offline verification, tampering, offline refusal of registry
packages, install order, cycles, duplicates, unresolved deps, and the strict
grammar.

## Next slices (proposed)

1. `[dependencies]` in `.atto.pkg.toml` plus a resolver (minimal version
   selection) that *produces* this lock. Today the lock is the input.
2. `attoc vendor` / `attoc build --offline`: read `vendor/<dir>` through the
   existing FD-rooted `secure_fs` and call `verify` before linking.
3. Feed locked dependency sources into `compile_linked` with the package
   identity `name@version` (already the linker's package key).
4. Signed lock digests (Sigstore/minisign) and a registry protocol, gated on
   a separate security review.
