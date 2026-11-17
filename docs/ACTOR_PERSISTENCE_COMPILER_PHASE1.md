# Atto actor persistence: Rust/browser compiler phase 1

**Status:** parser and scalar state-codec prototype added in this branch; no compiled guest disk persistence, no actor recovery, no generated Wasm snapshot entry points, and no full AOT backend parity.

This implementation follows the accepted design direction in:
- `atto-lang/atto-spec` draft PR #63
- `atto-lang/atto-common-source` draft PR #24
- `atto-lang/atto-platform-conformance` draft PR #62

## Lexical and AST behavior

```atto
define durable actor Counter as
    state count: int = 0;
end

define persistent actor Cart as
    state active: bool = false;
end
```

Both modifiers are mutually exclusive. The grammar accepts `define [pub] durable actor`, `define [pub] persistent actor`, and the older `pub define durable actor` surface. The parser records the storage mode explicitly on each `ActorDecl`. Plain actors remain volatile.

The `state name: Type = initializer;` form sets `FieldMode::ActorState` and is eligible for the existing actor isolation discipline. This frontend's executable actor fields currently admit **only `int` and `bool`**. Other types are not yet persistable here, even if the eventual cross-backend trait says they should be.

## Scalar codec prototype

`src/actor_persistence.rs` derives an immutable `ActorStateSchema` from an actor declaration. Its binary serialization is deterministic across host endianness:

1. 8-byte magic `ATTOST01`
2. 1-byte format version (currently 1)
3. 8-byte little-endian schema fingerprint, FNV-1a over canonical actor and typed field names
4. 2-byte little-endian number of fields
5. declaration-ordered field-tag + payload pairs (`1` + signed i64 LE or `2` + canonical 0/1 byte)
6. 8-byte FNV-1a checksum over all preceding bytes

The wire format is bounded to 4 KiB and 256 fields, and decoding rejects truncation, extra data, incorrect tags, illegal booleans, corruption and schema changes. FNV-1a is an accidental-corruption check, **not an adversarial authentication/MAC**. The runtime must supply an immutable, quiescent owned state value graph; this codec must never be invoked on a live actor while critical loans are active.

Future compiler-generated AOT codecs can use this versioned scalar representation as a conformance seed. A whole-object graph, cycles, nested collections, full migrations, pointer relocation and mailbox journaling are not implemented.

## Security gate

The browser type checker deliberately errors before Wasm emission:

- `E_PERSIST_HOOKS`: a `persistent actor` lacks user-defined `load_state` and `save_state`.
- `E_PERSIST_SIGNATURE`: the subset cannot yet type-check and execute generic suspended persistence hooks even when method names exist.
- `E_DURABLE_HOOK_OVERRIDE`: user attempts to implement storage hooks for a `durable actor`.
- `E_NON_PERSISTABLE_STATE`: a field has no proven safe codec.
- `E_DURABLE_RUNTIME_UNAVAILABLE`: no crash-safe on-disk runtime or revival exists here.
- `E_ACTOR_PERSISTENCE_MODIFIER`: duplicated modifiers or modifiers on non-actors.

There must never be an accepted, executable `durable actor` that is secretly emitted as a volatile actor. A successful offline roundtrip is **not** a checkpoint durability guarantee.

## Verification

`cargo test --locked --test actor_persistence_frontend` covers syntax, modifier diagnostics, fail-closed compilation, scalar roundtrip, tampering, truncation and incompatible schema. Check actual executed CI steps and exact Git SHA before calling tests verified. A workflow with no executed steps is **NOT_EXECUTED**, not passing evidence.

Next implementation slices: compile the full `ActorPersistence<State>` trait and suspended hooks; generate codec entry points from typed actor state; add an authorized SQLite WAL/FULL storage adapter for native workers; implement epoch-fenced commit and real post-SIGKILL revival; then expand types and verify backend parity.
