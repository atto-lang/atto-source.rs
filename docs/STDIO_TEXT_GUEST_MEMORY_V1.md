# Checked stdin guest-memory ownership (Rust/Wasm v1 draft)

**Status:** implementation staged on draft Rust PR #257; no compiler or Chromium
JSPI test receipt has executed for its current head. This is *not* tenant
isolation certification or permission to promote any parity status.

## Bounded scratch and typed results

The versioned `atto.stdio_text` import `stdio_text_read_v1` returns an
integer status and a 16-byte flat outcome, with a bounded **64 KiB**
UTF-8 destination. Guest source receives
`Result<Option<string>,IoError>`, not an unchecked `string`.
A status error becomes `Err(IoError)`; a flat Some becomes
`Ok(Some(text))`; clean EOF becomes `Ok(None)`. Empty records are
`Ok(Some(""))`, distinct from EOF.

A 16-page Wasm guest with checked stdin reserves its **last 64 KiB page**
for transient scratch. The bump allocator is bounded to the first **15
pages** for such modules. The checked read lowering uses a private
per-instance mutable `stdio_busy_global` lease to prevent a second
continuation from calling the host or overwriting scratch while an earlier
JSPI import is in flight **or before its bytes have been copied**.
Concurrent reads receive a checked `Err(ReaderBusy)` (numeric v1 code
17), not a shared view or ambient host handle.

After a successful host read, the compiler allocates only the
**actual byte length rounded up to an 8-byte boundary**, and performs a
Wasm `memory.copy` from scratch to this owned allocation. The returned
`Option<string>` points to the owned data; it never contains the shared
scratch address. The lease is released **before** invoking the cooperative
scheduler import on all return branches. This prevents the old
per-read 64 KiB *permanent* allocation and keeps previously returned
strings immutable across subsequent reads.

The existing heap remains a bounded bump arena. The 16-byte
Result/Option records and actual owned strings are **not garbage-collected
or automatically reclaimed** in this v1 subset. Long-running programs
can still reach the bounded heap limit. If a valid Some record cannot be
copied into remaining heap space, the lowering returns a typed
`Err(Quota)` (numeric v1 code 2), releases the scratch lease, and permits
a later small read. Heap capacity calculations and bump allocations use
unsigned, overflow-safe bounds; an overflowing size cannot wrap into
the reserved scratch page.

## Cancellation, security, limitations

- Only a trusted broker supplies `StdioTextHostV1` and a
  generation-matched, explicit per-port tenant grant. This file does not
  create a global `stdin` capability.
- The bridge checks memory ranges and disjoint result/data regions
  **before** reading input and **after** JSPI resumption; it does not
  retain a `DataView` across suspension.
- Aborting a host `readch({signal})` releases its input reader lease;
  bridge revocation settles pending stdin with `Revoked` and does not
  commit late input to guest memory. The underlying injected async
  iterator is *not* forcibly closed because the source may be
  process-owned.
- The per-instance scratch lease is released by the generated code on
  all ordinary checked outcomes (Some, None, source error, permission
  denial, Quota, and ReaderBusy). A malicious or broken host import
  **throwing an uncaught exception while suspended**, an out-of-memory
  trap allocating mandatory ADT records, or termination of the guest
  before resumption must be handled by destroying that TenantVM/instance;
  no claim is made that an abandoned Wasm frame can run a finally hook.
- A single 64 KiB scratch page limits a module to one outstanding
  checked stdin read, matching the explicit v1 single-consumer
  contract. Future concurrent input profiles require a separately
  versioned ABI, isolated buffers and proven cancellation cleanup.

## Required evidence

The staged Chromium browser Worker compiles *real .atto source* for
four old/new `on is` × readch/unary spellings, the exact nested
string-returning `Result<Option<string>, IoError>` match, **41
successive input reads** preserving a previously returned string, and
a large-record sequence that exhausts owned heap space then successfully
reads a small record after `Err(Quota)`. The Node JSPI import suite
checks that revoked reads never write late guest memory.

**These are tests to execute, not tests that passed.** Require pinned
Cargo, wasm32 compilation, Chromium JSPI guest-execution stdout/stderr,
negative no-artifact proofs and the exact Git tree before marking
Rust/Wasm or any other compiler as passing. Do not merge based on
zero-step GitHub Actions failures.
