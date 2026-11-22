# Native Atto actor event journal v1

**Status: native SQLite host implementation and local test evidence only.** This is a separate journal from the state **checkpoint** implementation in PR #266. It does not compile AttoLang actors, lower `decide/apply_event`, implement scheduler suspension, durable mailboxes, or supervisor-driven source guest recovery.

## Interface and committed semantics

`atto_actor_journal_v1.h/.c` provide a trusted native host ABI. The provider opens an application-authorized filesystem path, requires SQLite WAL and `synchronous=FULL`, and rejects `:memory:` and `file:` URI paths. An actor identifier is a **lookup key**, never a guest security capability; the host must derive and authorize the canonical tenant/namespace/type/instance identity before calling these APIs.

- `atto_event_journal_activate`: increment the persisted actor incarnation epoch, sharing `atto_actor_epochs` with the earlier checkpoint store in the **same database**.
- `atto_event_journal_append`: under `BEGIN IMMEDIATE`, check provider epoch **before** deduplicating; bind the operation ID to SHA-256 of the canonical owned command bytes and the original expected stream version; compare-and-swap the stream head; append a contiguous batch of immutable event payloads; write a receipt and any outbox entries in the **same SQL transaction**; commit before returning success.
- `atto_event_journal_resolve`: recover an original operation receipt after an ambiguous outcome using the same operation ID and original canonical command bytes. A conflicting digest fails; a timeout is not proof of rollback.
- `atto_event_journal_read`: bounded, ordered replay with sequence-gap and SHA-256 integrity checks. A failed read returns **no partially recovered event batch**.

The journal is capped at 32 events and 32 outbox records per command, 4096 bytes per event/outbox record, 65536 bytes total per batch, and 16384 command bytes. Each read page is capped at 64 events. Each actor state slot has a separate monotonic head; batches are atomic within the designated SQLite transaction.

## Build and run the native test

```sh
cd native/persistence_sqlite
cc -D_POSIX_C_SOURCE=200809L -std=c11 -Wall -Wextra -Werror -pedantic -g -O1 \
  -fsanitize=address,undefined -fno-omit-frame-pointer \
  atto_actor_sqlite_v1.c atto_actor_journal_v1.c test_actor_journal_v1.c \
  -lsqlite3 -lcrypto -o actor_journal_test
ASAN_OPTIONS=detect_leaks=1 UBSAN_OPTIONS=halt_on_error=1 ./actor_journal_test
```

The test includes two-event/one-outbox atomic commit and replay, deduplication, operation-ID/digest collision, stale stream CAS, epoch fencing shared with the checkpoint store, an injected outbox insertion failure that rolls back **events + head + operation receipt**, a tampered/missing journal entry, a real `SIGKILL` after 100 committed increments and pre-reply, a second process's revival to 100 and replay-safe retry, and a `SIGKILL` during an uncommitted write.

Locally executed on Linux with **GCC 14.2 and Clang 17** at **O1 and O2** under UBSan, plus **GCC and Clang O1** with ASan+UBSan; all runs returned success. Exact Git blob hashes are checked before using the local evidence. GitHub Actions run status must be reviewed separately, not inferred from local runs.

## Language surface: preserve AttoLang syntax

Event sourcing is an optional **trusted runtime policy** `#event_sourced` for an actor. The current canonical declarative shape is:

```atto
define durable actor BankAccount as
    state balance: int = 0;

    receive(ActorMail<int> mail): void {
        // Runtime-controlled event decision/commit will be compiled later.
    }
end
```

This is **target parser syntax**, not proof the browser compiler emits a durable actor. Do not introduce `define event actor`, invent configuration annotations, or bypass `handle_state(critical-read |s| -> ...)` / sealed-state authority. `persistent` still means developer-owned `load_state/save_state` storage hooks; `durable` still means runtime-managed disk storage. Event sourcing is distinct from a snapshot-only durability policy.

## Security/operational release gates still missing

- The native API trusts the caller's actor ID, state slot, operation ID, and canonical command bytes; **authentication, tenant authorization and scheduling authority belong to the host**. SHA-256 detects mutation/operation mismatch, but database modification by an attacker is outside this integrity model; authenticated storage/permissions are needed.
- No outbox relay, delivered-state acknowledgment, message journal, snapshot/journal offset coordination, schema migration or compaction is implemented. This code **writes** outbox rows atomically but does not dispatch them.
- No real AttoLang guest has compiled or executed this journal. No compiler-enforced pure `decide/apply_event` or borrow release at suspension, no supervisor lifecycle fencing of memory, no automatic post-crash guest revival, no five-backend parity.
- A standalone `SIGKILL` pass demonstrates process restart recovery but is **not** proof of power-loss durability on arbitrary disk hardware.
- An IO failure during commit may leave the result ambiguous. Callers MUST resolve the original operation before interpreting it as rolled back or retrying as a new operation.

See draft spec #65 and conformance #66. This PR should remain draft until actual executed GitHub CI and compiled actor/runtime evidence exist.
