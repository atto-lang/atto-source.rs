#ifndef ATTO_ACTOR_JOURNAL_V1_H
#define ATTO_ACTOR_JOURNAL_V1_H
#include <stddef.h>
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
/* Trusted native host ABI; NOT an executable Atto actor or generic compiler codec. */
typedef struct atto_event_journal atto_event_journal;
enum atto_event_status {
    ATTO_EVENT_OK = 0, ATTO_EVENT_ABSENT = 1,
    ATTO_EVENT_INVALID = -1, ATTO_EVENT_IO = -2,
    ATTO_EVENT_FENCED = -3, ATTO_EVENT_VERSION_CONFLICT = -4,
    ATTO_EVENT_OPERATION_CONFLICT = -5, ATTO_EVENT_CORRUPT = -6,
    ATTO_EVENT_LIMIT = -7
};
typedef struct { const void *bytes; size_t length; } atto_event_blob;
typedef struct {
    /* actor_id is a trusted canonical tenant/namespace/type/key, not guest authority. */
    const char *actor_id;
    const char *state_slot;
    uint64_t epoch;
    uint64_t expected_version;
    const char *operation_id;
    const void *command_bytes; /* Immutable canonical owned request bytes. */
    size_t command_length;
    const atto_event_blob *events;
    size_t event_count;
    const atto_event_blob *outbox;
    size_t outbox_count;
} atto_event_command;
typedef struct {
    uint64_t from_version, to_version;
    int replayed;
    unsigned char command_sha256[32];
} atto_event_receipt;
typedef struct { uint64_t sequence; void *bytes; size_t length; } atto_event_record;
/* Opens a trusted file path. Requires WAL + FULL; no :memory: or URI. */
int atto_event_journal_open(const char *trusted_path, atto_event_journal **out);
void atto_event_journal_close(atto_event_journal *journal);
/* Durable provider-side generation. Existing checkpoint store shares epochs table. */
int atto_event_journal_activate(atto_event_journal *journal, const char *actor_id, uint64_t *epoch);
/* All event blobs, dedupe receipt, and outbox payloads commit in ONE transaction. */
int atto_event_journal_append(atto_event_journal *journal, const atto_event_command *command,
                              atto_event_receipt *receipt);
/* After an unknown result, resolve by operation ID and original command bytes. */
int atto_event_journal_resolve(atto_event_journal *journal, const char *actor_id,
                               const char *state_slot, const char *operation_id,
                               const void *command_bytes, size_t command_length,
                               atto_event_receipt *receipt);
/* Read up to capacity contiguous events after 'after_version', validate hashes. */
int atto_event_journal_read(atto_event_journal *journal, const char *actor_id,
                            const char *state_slot, uint64_t after_version,
                            size_t capacity, atto_event_record **records_out,
                            size_t *count_out, uint64_t *head_version_out);
void atto_event_journal_free_records(atto_event_record *records, size_t count);
#ifdef __cplusplus
}
#endif
#endif
