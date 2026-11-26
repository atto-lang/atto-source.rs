#ifndef ATTO_ACTOR_SQLITE_V1_H
#define ATTO_ACTOR_SQLITE_V1_H
#include <stddef.h>
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
/* Native host checkpoint primitive, not actor scheduler or mailbox durability.
 * Each connection is owned by one privileged storage worker; callers must
 * serialize whole API calls (including close), not just individual SQL calls.
 * Independent connections coordinate via SQLite transactions. */
typedef struct atto_actor_db atto_actor_db;
/* Negative values distinguish missing, conflicting, corrupt and operational failures. */
enum atto_actor_store_status {
  ATTO_ACTOR_STORE_OK = 0,
  ATTO_ACTOR_STORE_ABSENT = 1,
  ATTO_ACTOR_STORE_INVALID = -1,
  ATTO_ACTOR_STORE_IO = -2,
  ATTO_ACTOR_STORE_STALE_EPOCH = -3,
  ATTO_ACTOR_STORE_STALE_SEQUENCE = -4,
  ATTO_ACTOR_STORE_SCHEMA = -5,
  ATTO_ACTOR_STORE_CORRUPT = -6,
  ATTO_ACTOR_STORE_RESOURCE = -7
};
/* Opens trusted application-configured path; requires SQLite WAL + FULL. */
int atto_actor_db_open(const char *trusted_path, atto_actor_db **out);
void atto_actor_db_close(atto_actor_db *db);
/* Legacy epoch-only activation; use activate_and_load for automatic revival. */
int atto_actor_activate(atto_actor_db *db, const char *actor_id, uint64_t *new_epoch);
/* The complete ATTOST01 versioned snapshot must have the given schema ID. */
int atto_actor_commit(atto_actor_db *db, const char *actor_id, uint64_t epoch,
                      uint64_t expected_sequence, uint64_t schema_id,
                      const void *snapshot, size_t snapshot_length,
                      uint64_t *new_sequence);
/* Legacy unfenced inspection, not for admitting an actor turn.
 * Returns malloc-allocated immutable bytes; call atto_actor_free. */
int atto_actor_load(atto_actor_db *db, const char *actor_id, uint64_t schema_id,
                    void **snapshot_out, size_t *snapshot_length_out,
                    uint64_t *sequence_out);
/* Atomically rotate epoch and load validated state. ABSENT means the new
 * epoch committed but no checkpoint exists; all negative statuses leave
 * outputs zero/null and roll back (subject to storage I/O ambiguity).
 * The broker must still fence execution/publication after this transaction. */
int atto_actor_activate_and_load(atto_actor_db *db, const char *actor_id, uint64_t schema_id,
                                 uint64_t *new_epoch, void **out, size_t *len, uint64_t *seq);
/* Restore only under the exact current epoch; stale owners receive no bytes,
 * including for an absent checkpoint. The read is transactionally fenced. */
int atto_actor_load_fenced(atto_actor_db *db, const char *actor_id, uint64_t epoch,
                           uint64_t schema_id, void **out, size_t *len, uint64_t *seq);
void atto_actor_free(void *ptr);
/* Parser for ATTOST01 scalar-only wire format and its integrity check. */
int atto_actor_validate_snapshot(const void *snapshot, size_t length, uint64_t schema_id);
#ifdef __cplusplus
}
#endif
#endif
