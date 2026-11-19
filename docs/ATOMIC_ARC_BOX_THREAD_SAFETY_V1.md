# AttoLang Atomic, Arc and Box: inter-thread safety contract (v1)

**Status:** specification and fail-closed browser frontend admission only.
This document does NOT certify an executable AttoLang `Atomic<T>`, `Arc<T>` or `Box<T>`
implementation. The browser Rust compiler rejects all three until a type-aware
ownership checker and target atomic/allocator lowering are available. Do not
advertise synthetic host-side examples as native guest coverage.

## Execution and authority model

A non-`serial` actor may execute multiple receiver methods concurrently, even
when all accesses originate from the same actor. Actor-owned memory and actor
identity do **not** imply that two receiver activations are mutually exclusive.
Distinct actor domains also cannot exchange physical actor-private pointers.
Send and Sync are compile-time worker/thread traits; actor mailbox Sendable,
serializability and cross-allocation-domain Share are separate decisions.

* `Send`: an exclusively owned value may MOVE to another concurrent lane.
* `Sync`: a shared borrow may be read by concurrent lanes without a data race.
* `Share`: shared allocation/domain capability is valid for the destination;
  not automatically derived from Send or Sync.
* No user-defined marker, FFI wrapper or nominal type spelling can assert these
  properties without compiler-trusted, backend-supported proofs.

| Intrinsic | Send proof | Sync proof | Actor boundary |
| --- | --- | --- | --- |
| `Box<T>` | `T: Send` | `T: Sync` for shared immutable borrows | MOVE only with validated arena transfer/relocation or serialization; never send a raw actor-private address |
| `Arc<T>` | `T: Send + Sync` | `T: Send + Sync` | additionally require shared allocator/domain authority, generations and teardown pins |
| `Atomic<T>` | supported, implemented atomically for `T` | supported, implemented atomically for `T` | not itself a cross-actor shared allocation |
| `Arc<Atomic<T>>` | both proofs plus target atomics | both proofs plus target atomics | explicit Share authority |
| `Arc<Mutex<T>>` | `T: Send` | `T: Send` with genuine guarded access | explicit Share and nonparking guarded lock implementation |
| `Rc<T>` | false | false | forbidden |

No `Copy` derivation for Box/Arc/Atomic: `Box` transfers uniquely, `Arc`
clones only through checked reference counting, and `Atomic` can be read via
`load` but duplicating the atomic object would create a different cell.

## Atomic<T> backend rules

Start with bool and integer scalar widths that the *specific target* can
implement with real atomic instructions or an audited, nonparking fallback.
Reject floating point, arbitrary records, raw addresses, and unsupported sizes
rather than lowering a nominal Atomic to ordinary memory. Never approximate
atomic read-modify-write with separate load+store.

Supported operations after runtime proof: `load`, `store`, `swap`,
`compare_exchange`, `compare_exchange_weak`, `fetch_add`,
`fetch_sub`, `fetch_and`, `fetch_or`, `fetch_xor` where type-appropriate.
Orderings: Relaxed, Acquire, Release, AcqRel, SeqCst.
Reject Release/AcqRel loads, Acquire/AcqRel stores, and any failure ordering
stronger than the CAS success ordering or with Release/AcqRel. Weak CAS callers
must handle spurious failure; checked arithmetic semantics apply to explicit
nonwrapping operations, while fetching arithmetic must explicitly specify
wrapping behavior. SeqCst is a safe default, not a claim of lock-freedom.
True inter-thread visibility must be validated across scheduler migration.

## Arc<T> lifetime rules

Clone increments strong count atomically; a failed increment on count overflow
aborts safely. Dropping an owner decrements with Release; the last strong
owner acquires before destroying T *exactly once*. Weak upgrade is a CAS of
the strong count conditional on nonzero, with Acquire on success and no
resurrection of a retired object. The control block is released only after
strong and weak counts and all permitted pins become quiescent. Stale
generation/epoch cannot upgrade or release a recycled allocation. Cycles
are not reclaimed without explicit Weak back-edges or another mechanism.
No shared mutable access via deref: mutation needs Atomic, Mutex or RwLock.
No Arc control block allocated inside a private actor arena may be published
to another actor.

## Box<T> move and drop rules

Box is a unique allocation owner with noncopyable provenance. A move
invalidates the source at type-check time; each branch, early return, panic,
cancellation and actor teardown must execute one drop at most. Shared and
mutable borrows obey read/write exclusivity, including suspension and nested
receiver calls. A Box cannot be implicitly cast to Arc or cloned.
A Box moved into another lane requires Send and valid ownership lifetime;
a Box moved across actor/private-heap boundaries needs relocation/transfer
or serialization, never pointer forwarding.

## Acceptance gates before enabling guest APIs

1. Typed generic intrinsic identity, exact field/argument trait derivation,
   invalid memory ordering diagnostics and borrow/capture checks, including
   inheritance, dynamic dispatch and FFI.
2. AOT lowering and binary ABI on all supported targets, with no GC thread
   and without assuming Java heap publication establishes actor sharing.
3. Deterministic 2/5/10/100-lane contention, CAS/ABA/reuse, refcount overflow,
   concurrent last-drop/weak-upgrade, exactly-once drop, after-move and
   suspension/teardown tests. TSan on native targets; ASan/UBSan for lifetime.
4. Cross-backend conformance with exact source tree SHA and genuine guest
   execution; negative cases must produce compile errors, never silent
   interpreter or host-side fallback.

Until those gates pass, the compiler must fail closed, even if a nominal
trait callback declares an unlowered intrinsic safe.
