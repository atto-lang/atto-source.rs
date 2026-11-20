# AttoLang native SQLite actor-state checkpoint store — v1

**Status:** tested standalone native storage component; **not** a compiled
AttoLang actor, auto-scheduled durable actor, persistent-hook implementation,
supervisor restart, or durable mailbox. The C ABI is intended for native runtimes
that will later connect the checked AOT actor-state codec to the runtime.

This storage provider implements `atto_actor_db_open`, `atto_actor_activate`,
`atto_actor_commit`, `atto_actor_load` and `atto_actor_free`.
The database uses SQLite `journal_mode=WAL` and `synchronous=FULL`; if WAL
cannot be selected it refuses to open. Only trusted paths are supported
(`file:` URI and `:memory:` are explicitly rejected). The caller must
enforce filesystem path permissions and capabilities. `actor_id` is a lookup
key, **not an authority token**.

Each activation increments a durable, monotonic actor incarnation epoch.
Commits use `BEGIN IMMEDIATE`, check that epoch and expected checkpoint
sequence, validate the `ATTOST01` format/schema/checksum, and publish
the new immutable snapshot and sequence in one SQL transaction. Loading
returns a heap copy after full validation, not an SQLite-owned pointer.
There is no silent fallback to in-memory storage.

This is the bounded v1 `int`/`bool` codec format from
`src/actor_persistence.rs` in the companion compiler PR #263. The runtime
must call these APIs only with a consistent quiescent actor-owned snapshot;
locks, sockets, continuations and raw pointers are **not** supported.
A checksum detects accidental corruption but is **not** a MAC. Schema
migration and encrypted stores are not implemented.

**Crucial durability limit:** state commits do not atomically consume or
acknowledge a durable mailbox message. No exactly-once delivery/replay or
external side-effect guarantee is implied. Fencing only protects this
database; a supervisor must separately fence old incarnations in memory.
A power-loss durability guarantee also depends on real filesystem/device
flush semantics, which cannot be proven by SIGKILL alone.

## Local native test

```sh
cd native/persistence_sqlite
cc -D_POSIX_C_SOURCE=200809L -std=c11 -O1 -g \
  -Wall -Wextra -Werror -pedantic \
  -fsanitize=address,undefined -fno-omit-frame-pointer \
  atto_actor_sqlite_v1.c test_actor_sqlite_v1.c \
  -lsqlite3 -o actor_sqlite_test
ASAN_OPTIONS=detect_leaks=1 ./actor_sqlite_test
```

The test forks real processes, waits until a SQLite transaction reaches
the checkpoint boundary, sends `SIGKILL`, reopens from a fresh connection,
and verifies commit survival or uncommitted rollback. It also covers
stale actor epochs, sequence CAS, schema mismatch and checksum corruption.
The test is native C with no Python dependency.

## Future integration gates

1. Native and browser compilers type-check and lower full `ActorPersistence<State>`
   `susp` hook signatures and runtime-generated `Persistable` codecs.
2. A host calls snapshot/restore only after a compiler-proven quiescent
   boundary; actor generation and supervisor state are fenced.
3. Runtime/journal atomically tracks admitted/processed mailbox messages
   alongside the state commit; no success ACK before barrier.
4. Real compiled actor guest sends 100 messages, process is killed, and a
   new native process revives the same logical ID and verifies state.
5. Repeat on the other AttoLang backends with exact compiled guest evidence.

## 100-message host revival demonstration

The second native test (`test_counter_revival_v1.c`) uses a small host-side
actor simulation: it admits and checkpoints 100 increments, kills the process,
opens a new SQLite connection, obtains a new incarnation epoch, verifies the
counter revives at 100, and commits increment 101. This tests the native
storage integration boundary, **not** an actual AttoLang-compiled actor.
