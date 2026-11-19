#define _POSIX_C_SOURCE 200809L
#include "atto_actor_journal_v1.h"
#include <limits.h>
#include <openssl/sha.h>
#include <sqlite3.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

/* Separate native journal surface, sharing the existing atto_actor_epochs
 * table with the checkpoint store on the SAME SQLite database. Caller owns
 * trusted authorization; a string actor ID must never be treated as a token.
 */
struct atto_event_journal { sqlite3 *db; };
#define MAX_KEY 255u
#define MAX_COMMAND 16384u
#define MAX_BLOB 4096u
#define MAX_ITEMS 32u
#define MAX_BATCH_BYTES 65536u
#define MAX_READ 64u

static int key_ok(const char *s) {
    return s && s[0] && strnlen(s, MAX_KEY + 1u) <= MAX_KEY;
}
static int exec_sql(sqlite3 *db, const char *sql) {
    return sqlite3_exec(db, sql, NULL, NULL, NULL) == SQLITE_OK;
}
static int rollback(sqlite3 *db, int status) {
    (void)sqlite3_exec(db, "ROLLBACK", NULL, NULL, NULL);
    return status;
}
static int prepared(sqlite3 *db, const char *sql, sqlite3_stmt **s) {
    return sqlite3_prepare_v2(db, sql, -1, s, NULL) == SQLITE_OK;
}
static int text_at(sqlite3_stmt *s, int i, const char *value) {
    return sqlite3_bind_text(s, i, value, -1, SQLITE_TRANSIENT) == SQLITE_OK;
}
static int actor_slot(sqlite3_stmt *s, const char *actor, const char *slot) {
    return text_at(s, 1, actor) && text_at(s, 2, slot);
}
static int bytes_at(sqlite3_stmt *s, int i, const void *bytes, size_t length) {
    return length <= INT_MAX &&
        sqlite3_bind_blob(s, i, bytes, (int)length, SQLITE_TRANSIENT) == SQLITE_OK;
}
static int sha256_of(const void *bytes, size_t n, unsigned char out[32]) {
    return bytes && n && SHA256((const unsigned char *)bytes, n, out) != NULL;
}
static int valid_blobs(const atto_event_blob *blobs, size_t n, size_t *bytes) {
    if (n > MAX_ITEMS || (n && !blobs)) return 0;
    for (size_t i = 0; i < n; ++i) {
        if (!blobs[i].bytes || blobs[i].length == 0 || blobs[i].length > MAX_BLOB ||
            *bytes > MAX_BATCH_BYTES - blobs[i].length) return 0;
        *bytes += blobs[i].length;
    }
    return 1;
}

int atto_event_journal_open(const char *path, atto_event_journal **out) {
    if (!out) return ATTO_EVENT_INVALID;
    *out = NULL;
    if (!path || !*path || strcmp(path, ":memory:") == 0 || strncmp(path, "file:", 5) == 0)
        return ATTO_EVENT_INVALID;
    atto_event_journal *journal = calloc(1, sizeof(*journal));
    if (!journal) return ATTO_EVENT_LIMIT;
    if (sqlite3_open_v2(path, &journal->db, SQLITE_OPEN_READWRITE |
                        SQLITE_OPEN_CREATE | SQLITE_OPEN_FULLMUTEX, NULL) != SQLITE_OK) goto fail;
    (void)sqlite3_busy_timeout(journal->db, 2500);
    if (!exec_sql(journal->db, "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;")) goto fail;
    sqlite3_stmt *s = NULL;
    if (!prepared(journal->db, "PRAGMA journal_mode;", &s)) goto fail;
    int rc = sqlite3_step(s);
    const unsigned char *mode = rc == SQLITE_ROW ? sqlite3_column_text(s, 0) : NULL;
    int wal = mode && strcmp((const char *)mode, "wal") == 0;
    sqlite3_finalize(s);
    if (!wal) goto fail;
    if (!prepared(journal->db, "PRAGMA synchronous;", &s)) goto fail;
    rc = sqlite3_step(s);
    int full = rc == SQLITE_ROW && sqlite3_column_int(s, 0) == 2;
    sqlite3_finalize(s);
    if (!full) goto fail;
    if (!exec_sql(journal->db,
        "CREATE TABLE IF NOT EXISTS atto_actor_epochs("
        "actor_id TEXT PRIMARY KEY NOT NULL,epoch INTEGER NOT NULL CHECK(epoch>0));"
        "CREATE TABLE IF NOT EXISTS atto_event_stream_heads("
        "actor_id TEXT NOT NULL,state_slot TEXT NOT NULL,head_version INTEGER NOT NULL CHECK(head_version>=0),"
        "PRIMARY KEY(actor_id,state_slot));"
        "CREATE TABLE IF NOT EXISTS atto_event_stream_events("
        "actor_id TEXT NOT NULL,state_slot TEXT NOT NULL,version INTEGER NOT NULL CHECK(version>0),"
        "payload BLOB NOT NULL CHECK(length(payload) BETWEEN 1 AND 4096),"
        "payload_sha256 BLOB NOT NULL CHECK(length(payload_sha256)=32),"
        "PRIMARY KEY(actor_id,state_slot,version),"
        "FOREIGN KEY(actor_id,state_slot) REFERENCES atto_event_stream_heads(actor_id,state_slot));"
        "CREATE TABLE IF NOT EXISTS atto_event_stream_operations("
        "actor_id TEXT NOT NULL,state_slot TEXT NOT NULL,operation_id TEXT NOT NULL,"
        "command_sha256 BLOB NOT NULL CHECK(length(command_sha256)=32),"
        "expected_version INTEGER NOT NULL CHECK(expected_version>=0),"
        "from_version INTEGER NOT NULL CHECK(from_version>=0),"
        "to_version INTEGER NOT NULL CHECK(to_version>from_version),"
        "PRIMARY KEY(actor_id,state_slot,operation_id),"
        "FOREIGN KEY(actor_id,state_slot) REFERENCES atto_event_stream_heads(actor_id,state_slot));"
        "CREATE TABLE IF NOT EXISTS atto_event_stream_outbox("
        "actor_id TEXT NOT NULL,state_slot TEXT NOT NULL,operation_id TEXT NOT NULL,"
        "item_index INTEGER NOT NULL CHECK(item_index>=0),"
        "payload BLOB NOT NULL CHECK(length(payload) BETWEEN 1 AND 4096),"
        "payload_sha256 BLOB NOT NULL CHECK(length(payload_sha256)=32),"
        "PRIMARY KEY(actor_id,state_slot,operation_id,item_index),"
        "FOREIGN KEY(actor_id,state_slot,operation_id) REFERENCES "
        "atto_event_stream_operations(actor_id,state_slot,operation_id));")) goto fail;
    *out = journal;
    return ATTO_EVENT_OK;
fail:
    if (journal->db) sqlite3_close(journal->db);
    free(journal);
    return ATTO_EVENT_IO;
}
void atto_event_journal_close(atto_event_journal *j) {
    if (!j) return;
    if (j->db) sqlite3_close(j->db);
    free(j);
}

int atto_event_journal_activate(atto_event_journal *j, const char *actor, uint64_t *out) {
    if (!j || !out || !key_ok(actor)) return ATTO_EVENT_INVALID;
    *out = 0;
    sqlite3 *db = j->db;
    if (!exec_sql(db, "BEGIN IMMEDIATE;")) return ATTO_EVENT_IO;
    sqlite3_stmt *s = NULL;
    if (!prepared(db, "SELECT epoch FROM atto_actor_epochs WHERE actor_id=?1;", &s))
        return rollback(db, ATTO_EVENT_IO);
    if (!text_at(s, 1, actor)) { sqlite3_finalize(s); return rollback(db, ATTO_EVENT_IO); }
    int rc = sqlite3_step(s);
    sqlite3_int64 epoch = 1;
    if (rc == SQLITE_ROW) {
        sqlite3_int64 prior = sqlite3_column_int64(s, 0);
        if (prior <= 0 || prior == INT64_MAX) { sqlite3_finalize(s); return rollback(db, ATTO_EVENT_LIMIT); }
        epoch = prior + 1;
    } else if (rc != SQLITE_DONE) { sqlite3_finalize(s); return rollback(db, ATTO_EVENT_IO); }
    sqlite3_finalize(s);
    if (!prepared(db, "INSERT INTO atto_actor_epochs(actor_id,epoch) VALUES(?1,?2) "
                      "ON CONFLICT(actor_id) DO UPDATE SET epoch=excluded.epoch;", &s))
        return rollback(db, ATTO_EVENT_IO);
    int ok = text_at(s, 1, actor) && sqlite3_bind_int64(s, 2, epoch) == SQLITE_OK;
    rc = ok ? sqlite3_step(s) : SQLITE_ERROR;
    sqlite3_finalize(s);
    if (rc != SQLITE_DONE) return rollback(db, ATTO_EVENT_IO);
    if (!exec_sql(db, "COMMIT;")) return rollback(db, ATTO_EVENT_IO);
    *out = (uint64_t)epoch;
    return ATTO_EVENT_OK;
}

/* Check caller's epoch first, then deduplication, then expected stream version.
 * Must execute under BEGIN IMMEDIATE so no concurrent commit can race. */
static int check_epoch(sqlite3 *db, const char *actor, uint64_t epoch) {
    sqlite3_stmt *s = NULL;
    if (!prepared(db, "SELECT epoch FROM atto_actor_epochs WHERE actor_id=?1;", &s)) return ATTO_EVENT_IO;
    if (!text_at(s, 1, actor)) { sqlite3_finalize(s); return ATTO_EVENT_IO; }
    int rc = sqlite3_step(s);
    int status = rc == SQLITE_ROW ?
        (sqlite3_column_int64(s, 0) == (sqlite3_int64)epoch ? ATTO_EVENT_OK : ATTO_EVENT_FENCED) :
        (rc == SQLITE_DONE ? ATTO_EVENT_FENCED : ATTO_EVENT_IO);
    sqlite3_finalize(s);
    return status;
}
static int prior_receipt(sqlite3 *db, const char *actor, const char *slot,
                         const char *operation, const unsigned char digest[32],
                         uint64_t expected, atto_event_receipt *out) {
    sqlite3_stmt *s = NULL;
    if (!prepared(db, "SELECT command_sha256,expected_version,from_version,to_version FROM "
                      "atto_event_stream_operations WHERE actor_id=?1 AND state_slot=?2 "
                      "AND operation_id=?3;", &s)) return ATTO_EVENT_IO;
    if (!actor_slot(s, actor, slot) || !text_at(s, 3, operation)) {
        sqlite3_finalize(s); return ATTO_EVENT_IO;
    }
    int rc = sqlite3_step(s);
    if (rc == SQLITE_DONE) { sqlite3_finalize(s); return ATTO_EVENT_ABSENT; }
    if (rc != SQLITE_ROW) { sqlite3_finalize(s); return ATTO_EVENT_IO; }
    const void *stored = sqlite3_column_blob(s, 0);
    sqlite3_int64 exp = sqlite3_column_int64(s, 1);
    sqlite3_int64 from = sqlite3_column_int64(s, 2);
    sqlite3_int64 to = sqlite3_column_int64(s, 3);
    int valid = sqlite3_column_bytes(s, 0) == 32 && stored && exp >= 0 &&
        from >= 0 && to > from && to <= INT64_MAX;
    int same = valid && memcmp(stored, digest, 32) == 0 &&
        exp == (sqlite3_int64)expected;
    sqlite3_finalize(s);
    if (!valid) return ATTO_EVENT_CORRUPT;
    if (!same) return ATTO_EVENT_OPERATION_CONFLICT;
    out->from_version = (uint64_t)from;
    out->to_version = (uint64_t)to;
    out->replayed = 1;
    memcpy(out->command_sha256, digest, 32);
    return ATTO_EVENT_OK;
}

int atto_event_journal_append(atto_event_journal *j, const atto_event_command *c,
                              atto_event_receipt *receipt) {
    if (!j || !c || !receipt || !key_ok(c->actor_id) || !key_ok(c->state_slot) ||
        !key_ok(c->operation_id) || c->epoch == 0 || c->epoch > INT64_MAX ||
        c->expected_version > INT64_MAX || !c->command_bytes ||
        c->command_length == 0 || c->command_length > MAX_COMMAND ||
        c->event_count == 0 || c->event_count > MAX_ITEMS ||
        c->outbox_count > MAX_ITEMS ||
        c->expected_version > (uint64_t)INT64_MAX - c->event_count)
        return ATTO_EVENT_INVALID;
    size_t batch_size = 0;
    if (!valid_blobs(c->events, c->event_count, &batch_size) ||
        !valid_blobs(c->outbox, c->outbox_count, &batch_size)) return ATTO_EVENT_LIMIT;
    unsigned char digest[32];
    if (!sha256_of(c->command_bytes, c->command_length, digest)) return ATTO_EVENT_INVALID;
    memset(receipt, 0, sizeof(*receipt));
    sqlite3 *db = j->db;
    if (!exec_sql(db, "BEGIN IMMEDIATE;")) return ATTO_EVENT_IO;
    int status = check_epoch(db, c->actor_id, c->epoch);
    if (status != ATTO_EVENT_OK) return rollback(db, status);
    status = prior_receipt(db, c->actor_id, c->state_slot, c->operation_id,
                           digest, c->expected_version, receipt);
    if (status == ATTO_EVENT_OK) {
        if (!exec_sql(db, "COMMIT;")) return rollback(db, ATTO_EVENT_IO);
        return ATTO_EVENT_OK;
    }
    if (status != ATTO_EVENT_ABSENT) return rollback(db, status);
    sqlite3_stmt *s = NULL;
    if (!prepared(db, "SELECT head_version FROM atto_event_stream_heads WHERE actor_id=?1 "
                      "AND state_slot=?2;", &s)) return rollback(db, ATTO_EVENT_IO);
    if (!actor_slot(s, c->actor_id, c->state_slot)) { sqlite3_finalize(s); return rollback(db, ATTO_EVENT_IO); }
    int rc = sqlite3_step(s);
    uint64_t head = 0;
    if (rc == SQLITE_ROW) {
        sqlite3_int64 value = sqlite3_column_int64(s, 0);
        if (value < 0) { sqlite3_finalize(s); return rollback(db, ATTO_EVENT_CORRUPT); }
        head = (uint64_t)value;
    } else if (rc != SQLITE_DONE) { sqlite3_finalize(s); return rollback(db, ATTO_EVENT_IO); }
    sqlite3_finalize(s);
    if (head != c->expected_version) return rollback(db, ATTO_EVENT_VERSION_CONFLICT);
    uint64_t next = head + c->event_count;
    if (!prepared(db, "INSERT INTO atto_event_stream_heads(actor_id,state_slot,head_version) "
                      "VALUES(?1,?2,?3) ON CONFLICT(actor_id,state_slot) DO UPDATE SET "
                      "head_version=excluded.head_version;", &s)) return rollback(db, ATTO_EVENT_IO);
    int ok = actor_slot(s, c->actor_id, c->state_slot) &&
        sqlite3_bind_int64(s, 3, (sqlite3_int64)next) == SQLITE_OK;
    rc = ok ? sqlite3_step(s) : SQLITE_ERROR;
    sqlite3_finalize(s);
    if (rc != SQLITE_DONE) return rollback(db, ATTO_EVENT_IO);
    if (!prepared(db, "INSERT INTO atto_event_stream_events(actor_id,state_slot,version,payload,payload_sha256) "
                      "VALUES(?1,?2,?3,?4,?5);", &s)) return rollback(db, ATTO_EVENT_IO);
    for (size_t i = 0; i < c->event_count; i++) {
        unsigned char hash[32];
        if (!sha256_of(c->events[i].bytes, c->events[i].length, hash)) {
            sqlite3_finalize(s); return rollback(db, ATTO_EVENT_INVALID);
        }
        sqlite3_reset(s); sqlite3_clear_bindings(s);
        ok = actor_slot(s, c->actor_id, c->state_slot) &&
            sqlite3_bind_int64(s, 3, (sqlite3_int64)(head + i + 1)) == SQLITE_OK &&
            bytes_at(s, 4, c->events[i].bytes, c->events[i].length) && bytes_at(s, 5, hash, 32);
        if (!ok || sqlite3_step(s) != SQLITE_DONE) { sqlite3_finalize(s); return rollback(db, ATTO_EVENT_IO); }
    }
    sqlite3_finalize(s);
    if (!prepared(db, "INSERT INTO atto_event_stream_operations(actor_id,state_slot,operation_id,"
                      "command_sha256,expected_version,from_version,to_version) "
                      "VALUES(?1,?2,?3,?4,?5,?6,?7);", &s)) return rollback(db, ATTO_EVENT_IO);
    ok = actor_slot(s, c->actor_id, c->state_slot) && text_at(s, 3, c->operation_id) &&
        bytes_at(s, 4, digest, 32) &&
        sqlite3_bind_int64(s, 5, (sqlite3_int64)c->expected_version) == SQLITE_OK &&
        sqlite3_bind_int64(s, 6, (sqlite3_int64)head) == SQLITE_OK &&
        sqlite3_bind_int64(s, 7, (sqlite3_int64)next) == SQLITE_OK;
    rc = ok ? sqlite3_step(s) : SQLITE_ERROR;
    sqlite3_finalize(s);
    if (rc != SQLITE_DONE) return rollback(db, ATTO_EVENT_IO);
    if (c->outbox_count) {
        if (!prepared(db, "INSERT INTO atto_event_stream_outbox(actor_id,state_slot,operation_id,"
                          "item_index,payload,payload_sha256) VALUES(?1,?2,?3,?4,?5,?6);", &s))
            return rollback(db, ATTO_EVENT_IO);
        for (size_t i = 0; i < c->outbox_count; i++) {
            unsigned char hash[32];
            if (!sha256_of(c->outbox[i].bytes, c->outbox[i].length, hash)) {
                sqlite3_finalize(s); return rollback(db, ATTO_EVENT_INVALID);
            }
            sqlite3_reset(s); sqlite3_clear_bindings(s);
            ok = actor_slot(s, c->actor_id, c->state_slot) && text_at(s, 3, c->operation_id) &&
                sqlite3_bind_int64(s, 4, (sqlite3_int64)i) == SQLITE_OK &&
                bytes_at(s, 5, c->outbox[i].bytes, c->outbox[i].length) && bytes_at(s, 6, hash, 32);
            if (!ok || sqlite3_step(s) != SQLITE_DONE) { sqlite3_finalize(s); return rollback(db, ATTO_EVENT_IO); }
        }
        sqlite3_finalize(s);
    }
    if (!exec_sql(db, "COMMIT;")) return rollback(db, ATTO_EVENT_IO);
    receipt->from_version = head;
    receipt->to_version = next;
    receipt->replayed = 0;
    memcpy(receipt->command_sha256, digest, 32);
    return ATTO_EVENT_OK;
}

int atto_event_journal_resolve(atto_event_journal *j, const char *actor,
                               const char *slot, const char *operation,
                               const void *command, size_t n, atto_event_receipt *receipt) {
    if (!j || !receipt || !key_ok(actor) || !key_ok(slot) || !key_ok(operation) ||
        !command || n == 0 || n > MAX_COMMAND) return ATTO_EVENT_INVALID;
    unsigned char digest[32];
    if (!sha256_of(command, n, digest)) return ATTO_EVENT_INVALID;
    memset(receipt, 0, sizeof(*receipt));
    /* Exact operation ID and original command digest, no guess from timeout. */
    sqlite3_stmt *s = NULL;
    if (!prepared(j->db, "SELECT expected_version FROM atto_event_stream_operations "
                         "WHERE actor_id=?1 AND state_slot=?2 AND operation_id=?3;", &s))
        return ATTO_EVENT_IO;
    if (!actor_slot(s, actor, slot) || !text_at(s, 3, operation)) {
        sqlite3_finalize(s); return ATTO_EVENT_IO;
    }
    int rc = sqlite3_step(s);
    if (rc == SQLITE_DONE) { sqlite3_finalize(s); return ATTO_EVENT_ABSENT; }
    if (rc != SQLITE_ROW) { sqlite3_finalize(s); return ATTO_EVENT_IO; }
    sqlite3_int64 exp = sqlite3_column_int64(s, 0);
    sqlite3_finalize(s);
    if (exp < 0) return ATTO_EVENT_CORRUPT;
    return prior_receipt(j->db, actor, slot, operation, digest, (uint64_t)exp, receipt);
}

void atto_event_journal_free_records(atto_event_record *records, size_t n) {
    if (!records) return;
    for (size_t i = 0; i < n; i++) free(records[i].bytes);
    free(records);
}
int atto_event_journal_read(atto_event_journal *j, const char *actor,
                            const char *slot, uint64_t after, size_t capacity,
                            atto_event_record **out, size_t *out_count, uint64_t *out_head) {
    if (!j || !key_ok(actor) || !key_ok(slot) || !out || !out_count || !out_head ||
        capacity == 0 || capacity > MAX_READ || after > INT64_MAX) return ATTO_EVENT_INVALID;
    *out = NULL; *out_count = 0; *out_head = 0;
    sqlite3 *db = j->db;
    if (!exec_sql(db, "BEGIN;")) return ATTO_EVENT_IO;
    sqlite3_stmt *s = NULL;
    if (!prepared(db, "SELECT head_version FROM atto_event_stream_heads WHERE actor_id=?1 "
                      "AND state_slot=?2;", &s)) return rollback(db, ATTO_EVENT_IO);
    if (!actor_slot(s, actor, slot)) { sqlite3_finalize(s); return rollback(db, ATTO_EVENT_IO); }
    int rc = sqlite3_step(s);
    if (rc == SQLITE_DONE) { sqlite3_finalize(s); return rollback(db, ATTO_EVENT_ABSENT); }
    if (rc != SQLITE_ROW) { sqlite3_finalize(s); return rollback(db, ATTO_EVENT_IO); }
    sqlite3_int64 h = sqlite3_column_int64(s, 0);
    sqlite3_finalize(s);
    if (h < 0 || (uint64_t)h < after) return rollback(db, ATTO_EVENT_CORRUPT);
    *out_head = (uint64_t)h;
    size_t expected = (size_t)(((uint64_t)h - after) < capacity ? ((uint64_t)h - after) : capacity);
    atto_event_record *records = calloc(capacity, sizeof(*records));
    if (!records) return rollback(db, ATTO_EVENT_LIMIT);
    if (!prepared(db, "SELECT version,payload,payload_sha256 FROM atto_event_stream_events "
                      "WHERE actor_id=?1 AND state_slot=?2 AND version>?3 "
                      "ORDER BY version ASC LIMIT ?4;", &s)) {
        free(records); return rollback(db, ATTO_EVENT_IO);
    }
    if (!actor_slot(s, actor, slot) ||
        sqlite3_bind_int64(s, 3, (sqlite3_int64)after) != SQLITE_OK ||
        sqlite3_bind_int64(s, 4, (sqlite3_int64)capacity) != SQLITE_OK) {
        sqlite3_finalize(s); free(records); return rollback(db, ATTO_EVENT_IO);
    }
    size_t count = 0;
    int status = ATTO_EVENT_OK;
    while ((rc = sqlite3_step(s)) == SQLITE_ROW) {
        if (count >= expected || sqlite3_column_int64(s, 0) != (sqlite3_int64)(after + count + 1)) {
            status = ATTO_EVENT_CORRUPT; break;
        }
        int n = sqlite3_column_bytes(s, 1);
        const void *payload = sqlite3_column_blob(s, 1);
        const void *digest = sqlite3_column_blob(s, 2);
        unsigned char verify[32];
        if (n < 1 || n > (int)MAX_BLOB || !payload || !digest ||
            sqlite3_column_bytes(s, 2) != 32 ||
            !sha256_of(payload, (size_t)n, verify) || memcmp(digest, verify, 32)) {
            status = ATTO_EVENT_CORRUPT; break;
        }
        records[count].bytes = malloc((size_t)n);
        if (!records[count].bytes) { status = ATTO_EVENT_LIMIT; break; }
        memcpy(records[count].bytes, payload, (size_t)n);
        records[count].sequence = after + count + 1;
        records[count].length = (size_t)n;
        count++;
    }
    if (rc != SQLITE_DONE && status == ATTO_EVENT_OK) status = ATTO_EVENT_IO;
    if (status == ATTO_EVENT_OK && count != expected) status = ATTO_EVENT_CORRUPT;
    sqlite3_finalize(s);
    (void)sqlite3_exec(db, "ROLLBACK", NULL, NULL, NULL);
    if (status != ATTO_EVENT_OK) {
        atto_event_journal_free_records(records, capacity);
        *out_head = 0;
        return status;
    }
    *out = records;
    *out_count = count;
    return ATTO_EVENT_OK;
}
