#include "atto_actor_sqlite_v1.h"
#include <limits.h>
#include <sqlite3.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

/* SQLite-backed checkpoint store ABI v1. Not actor runtime integration.
 * Caller must hold trusted actor authority; actor_id is NOT a capability.
 * A native host must ensure a quiescent, independent state snapshot.
 */
struct atto_actor_db { sqlite3 *conn; };
static const unsigned char MAGIC[8] = {'A','T','T','O','S','T','0','1'};
#define MAX_SNAPSHOT_BYTES 4096u
#define MAX_FIELDS 256u
#define MAX_ACTOR_ID 255u

static uint64_t le64(const unsigned char *p) {
    uint64_t x=0;
    for (unsigned i=0; i<8; i++) x |= (uint64_t)p[i] << (8*i);
    return x;
}
static uint16_t le16(const unsigned char *p) {
    return (uint16_t)((uint16_t)p[0] | (uint16_t)((uint16_t)p[1] << 8));
}
static uint64_t fnv64(const unsigned char *b, size_t length) {
    uint64_t h = UINT64_C(0xcbf29ce484222325);
    for (size_t i=0; i<length; i++) { h ^= b[i]; h *= UINT64_C(0x100000001b3); }
    return h;
}
static int id_ok(const char *id) {
    if (!id) return 0;
    size_t n=strnlen(id, MAX_ACTOR_ID+1u);
    return n>0 && n<=MAX_ACTOR_ID;
}
static int sql_exec(sqlite3 *db, const char *sql) { return sqlite3_exec(db, sql, NULL,NULL,NULL)==SQLITE_OK; }
static int abort_tx(sqlite3 *db, int code) { (void)sqlite3_exec(db,"ROLLBACK",NULL,NULL,NULL); return code; }

int atto_actor_validate_snapshot(const void *snapshot, size_t length, uint64_t schema_id) {
    if (!snapshot || length<27u || length>MAX_SNAPSHOT_BYTES) return ATTO_ACTOR_STORE_INVALID;
    const unsigned char *p=(const unsigned char*)snapshot;
    if (memcmp(p,MAGIC,8)!=0 || p[8]!=1) return ATTO_ACTOR_STORE_CORRUPT;
    if (le64(p+9)!=schema_id) return ATTO_ACTOR_STORE_SCHEMA;
    unsigned count=le16(p+17);
    if (count>MAX_FIELDS) return ATTO_ACTOR_STORE_CORRUPT;
    const size_t end=length-8u;
    if (fnv64(p,end)!=le64(p+end)) return ATTO_ACTOR_STORE_CORRUPT;
    size_t cursor=19;
    for (unsigned i=0; i<count; i++) {
        if (cursor>=end) return ATTO_ACTOR_STORE_CORRUPT;
        const unsigned char tag=p[cursor++];
        const size_t bytes=(tag==1)?8u:((tag==2)?1u:0u);
        if (bytes==0u || bytes>end-cursor) return ATTO_ACTOR_STORE_CORRUPT;
        if (tag==2 && p[cursor]>1) return ATTO_ACTOR_STORE_CORRUPT;
        cursor+=bytes;
    }
    return cursor==end?ATTO_ACTOR_STORE_OK:ATTO_ACTOR_STORE_CORRUPT;
}

static int prep(sqlite3 *db,const char *sql,sqlite3_stmt **out) {
    return sqlite3_prepare_v2(db,sql,-1,out,NULL)==SQLITE_OK;
}
static int bind_id(sqlite3_stmt *st,const char *actor_id) {
    return sqlite3_bind_text(st,1,actor_id,-1,SQLITE_TRANSIENT)==SQLITE_OK;
}
static void schema_bytes(uint64_t schema_id,unsigned char out[8]) {
    for (unsigned i=0;i<8;i++) out[i]=(unsigned char)(schema_id>>(8*i));
}
int atto_actor_db_open(const char *trusted_path, atto_actor_db **out) {
    if (!out) return ATTO_ACTOR_STORE_INVALID;
    *out=NULL;
    if (!trusted_path || !*trusted_path || strcmp(trusted_path,":memory:")==0 ||
        strncmp(trusted_path,"file:",5)==0) return ATTO_ACTOR_STORE_INVALID;
    atto_actor_db *store=calloc(1,sizeof(*store));
    if (!store) return ATTO_ACTOR_STORE_RESOURCE;
    int rc=sqlite3_open_v2(trusted_path,&store->conn,SQLITE_OPEN_READWRITE|SQLITE_OPEN_CREATE|SQLITE_OPEN_FULLMUTEX,NULL);
    if (rc!=SQLITE_OK) goto fail;
    (void)sqlite3_busy_timeout(store->conn,2500);
    if (!sql_exec(store->conn,"PRAGMA journal_mode=WAL;")) goto fail;
    sqlite3_stmt *p=NULL;
    if (!prep(store->conn,"PRAGMA journal_mode;",&p)) goto fail;
    rc=sqlite3_step(p);
    const unsigned char *mode=rc==SQLITE_ROW?sqlite3_column_text(p,0):NULL;
    int is_wal=mode && strcmp((const char*)mode,"wal")==0;
    sqlite3_finalize(p);
    if (!is_wal) goto fail;
    if (!sql_exec(store->conn,"PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;")) goto fail;
    if (!sql_exec(store->conn,
      "CREATE TABLE IF NOT EXISTS atto_actor_epochs (actor_id TEXT PRIMARY KEY NOT NULL, epoch INTEGER NOT NULL CHECK(epoch>0));"
      "CREATE TABLE IF NOT EXISTS atto_actor_snapshots ("
      "actor_id TEXT PRIMARY KEY NOT NULL REFERENCES atto_actor_epochs(actor_id),"
      "sequence INTEGER NOT NULL CHECK(sequence>0), schema_tag BLOB NOT NULL CHECK(length(schema_tag)=8),"
      "snapshot BLOB NOT NULL CHECK(length(snapshot)<=4096 AND length(snapshot)>=27));")) goto fail;
    *out=store;
    return ATTO_ACTOR_STORE_OK;
fail:
    if (store->conn) sqlite3_close(store->conn);
    free(store);
    return ATTO_ACTOR_STORE_IO;
}
void atto_actor_db_close(atto_actor_db *db) {
    if (!db) return;
    if (db->conn) sqlite3_close(db->conn);
    free(db);
}
void atto_actor_free(void *ptr) { free(ptr); }

int atto_actor_activate(atto_actor_db *store, const char *actor_id, uint64_t *new_epoch) {
    if (!store || !new_epoch || !id_ok(actor_id)) return ATTO_ACTOR_STORE_INVALID;
    *new_epoch=0;
    sqlite3 *db=store->conn;
    if (!sql_exec(db,"BEGIN IMMEDIATE;")) return ATTO_ACTOR_STORE_IO;
    sqlite3_stmt *st=NULL;
    if (!prep(db,"SELECT epoch FROM atto_actor_epochs WHERE actor_id=?1;",&st)) return abort_tx(db,ATTO_ACTOR_STORE_IO);
    (void)bind_id(st,actor_id);
    int rc=sqlite3_step(st);
    sqlite3_int64 epoch=0;
    if (rc==SQLITE_ROW) {
        epoch=sqlite3_column_int64(st,0);
        if (epoch<1 || epoch==INT64_MAX) {sqlite3_finalize(st);return abort_tx(db,ATTO_ACTOR_STORE_RESOURCE);}
        epoch++;
    } else if (rc==SQLITE_DONE) { epoch=1; }
    else {sqlite3_finalize(st);return abort_tx(db,ATTO_ACTOR_STORE_IO);}
    sqlite3_finalize(st);
    if (!prep(db,"INSERT INTO atto_actor_epochs(actor_id,epoch) VALUES(?1,?2) "
                 "ON CONFLICT(actor_id) DO UPDATE SET epoch=excluded.epoch;",&st))
        return abort_tx(db,ATTO_ACTOR_STORE_IO);
    (void)bind_id(st,actor_id);
    (void)sqlite3_bind_int64(st,2,epoch);
    rc=sqlite3_step(st);
    sqlite3_finalize(st);
    if (rc!=SQLITE_DONE) return abort_tx(db,ATTO_ACTOR_STORE_IO);
    if (!sql_exec(db,"COMMIT;")) return abort_tx(db,ATTO_ACTOR_STORE_IO);
    *new_epoch=(uint64_t)epoch;
    return ATTO_ACTOR_STORE_OK;
}

int atto_actor_commit(atto_actor_db *store, const char *actor_id, uint64_t epoch,
                      uint64_t expected_sequence, uint64_t schema_id,
                      const void *snapshot, size_t snapshot_length,
                      uint64_t *new_sequence) {
    if (!store || !new_sequence || !id_ok(actor_id) || epoch==0 || epoch>INT64_MAX ||
        expected_sequence>=INT64_MAX || snapshot_length>MAX_SNAPSHOT_BYTES)
        return ATTO_ACTOR_STORE_INVALID;
    *new_sequence=0;
    int valid=atto_actor_validate_snapshot(snapshot,snapshot_length,schema_id);
    if (valid!=ATTO_ACTOR_STORE_OK) return valid;
    sqlite3 *db=store->conn;
    if (!sql_exec(db,"BEGIN IMMEDIATE;")) return ATTO_ACTOR_STORE_IO;
    sqlite3_stmt *st=NULL;
    if (!prep(db,"SELECT epoch FROM atto_actor_epochs WHERE actor_id=?1;",&st)) return abort_tx(db,ATTO_ACTOR_STORE_IO);
    (void)bind_id(st,actor_id);
    int rc=sqlite3_step(st);
    int current=rc==SQLITE_ROW && sqlite3_column_int64(st,0)==(sqlite3_int64)epoch;
    sqlite3_finalize(st);
    if (!current) return abort_tx(db,rc==SQLITE_ROW||rc==SQLITE_DONE?ATTO_ACTOR_STORE_STALE_EPOCH:ATTO_ACTOR_STORE_IO);
    if (!prep(db,"SELECT sequence FROM atto_actor_snapshots WHERE actor_id=?1;",&st)) return abort_tx(db,ATTO_ACTOR_STORE_IO);
    (void)bind_id(st,actor_id);
    rc=sqlite3_step(st);
    uint64_t sequence=0;
    if (rc==SQLITE_ROW) {
        sqlite3_int64 old=sqlite3_column_int64(st,0);
        if (old<1) {sqlite3_finalize(st);return abort_tx(db,ATTO_ACTOR_STORE_CORRUPT);}
        sequence=(uint64_t)old;
    } else if (rc!=SQLITE_DONE) {sqlite3_finalize(st);return abort_tx(db,ATTO_ACTOR_STORE_IO);}
    sqlite3_finalize(st);
    if (expected_sequence!=sequence) return abort_tx(db,ATTO_ACTOR_STORE_STALE_SEQUENCE);
    unsigned char tag[8];schema_bytes(schema_id,tag);
    if (!prep(db,"INSERT INTO atto_actor_snapshots(actor_id,sequence,schema_tag,snapshot) VALUES(?1,?2,?3,?4) "
                 "ON CONFLICT(actor_id) DO UPDATE SET sequence=excluded.sequence,schema_tag=excluded.schema_tag,snapshot=excluded.snapshot;",&st))
        return abort_tx(db,ATTO_ACTOR_STORE_IO);
    (void)bind_id(st,actor_id);
    (void)sqlite3_bind_int64(st,2,(sqlite3_int64)(sequence+1));
    (void)sqlite3_bind_blob(st,3,tag,8,SQLITE_TRANSIENT);
    (void)sqlite3_bind_blob(st,4,snapshot,(int)snapshot_length,SQLITE_TRANSIENT);
    rc=sqlite3_step(st);
    sqlite3_finalize(st);
    if (rc!=SQLITE_DONE) return abort_tx(db,ATTO_ACTOR_STORE_IO);
    if (!sql_exec(db,"COMMIT;")) return abort_tx(db,ATTO_ACTOR_STORE_IO);
    *new_sequence=sequence+1;
    return ATTO_ACTOR_STORE_OK;
}

int atto_actor_load(atto_actor_db *store, const char *actor_id, uint64_t schema_id,
                    void **snapshot_out, size_t *snapshot_length_out, uint64_t *sequence_out) {
    if (!store || !id_ok(actor_id) || !snapshot_out || !snapshot_length_out || !sequence_out)
        return ATTO_ACTOR_STORE_INVALID;
    *snapshot_out=NULL; *snapshot_length_out=0; *sequence_out=0;
    sqlite3_stmt *st=NULL;
    if (!prep(store->conn,"SELECT schema_tag,snapshot,sequence FROM atto_actor_snapshots WHERE actor_id=?1;",&st))
        return ATTO_ACTOR_STORE_IO;
    (void)bind_id(st,actor_id);
    int rc=sqlite3_step(st);
    if (rc==SQLITE_DONE) {sqlite3_finalize(st);return ATTO_ACTOR_STORE_ABSENT;}
    if (rc!=SQLITE_ROW) {sqlite3_finalize(st);return ATTO_ACTOR_STORE_IO;}
    const void *tag=sqlite3_column_blob(st,0);
    const void *data=sqlite3_column_blob(st,1);
    int tlen=sqlite3_column_bytes(st,0),length=sqlite3_column_bytes(st,1);
    sqlite3_int64 seq=sqlite3_column_int64(st,2);
    unsigned char expected[8];schema_bytes(schema_id,expected);
    if (tlen!=8 || !tag || memcmp(tag,expected,8)!=0) {sqlite3_finalize(st);return ATTO_ACTOR_STORE_SCHEMA;}
    if (length<27 || length>(int)MAX_SNAPSHOT_BYTES || !data || seq<1) {
        sqlite3_finalize(st);return ATTO_ACTOR_STORE_CORRUPT;
    }
    int valid=atto_actor_validate_snapshot(data,(size_t)length,schema_id);
    if (valid!=ATTO_ACTOR_STORE_OK) {sqlite3_finalize(st);return valid;}
    void *copy=malloc((size_t)length);
    if (!copy) {sqlite3_finalize(st);return ATTO_ACTOR_STORE_RESOURCE;}
    memcpy(copy,data,(size_t)length);
    sqlite3_finalize(st);
    *snapshot_out=copy; *snapshot_length_out=(size_t)length; *sequence_out=(uint64_t)seq;
    return ATTO_ACTOR_STORE_OK;
}
