#define _POSIX_C_SOURCE 200809L
#include "atto_actor_journal_v1.h"
#include "atto_actor_sqlite_v1.h"
#include <assert.h>
#include <limits.h>
#include <sqlite3.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

static void expect(int got, int want, const char *why) {
    if (got != want) { fprintf(stderr,"FAIL %s: got %d want %d\n",why,got,want); exit(7); }
}
static int64_t sql_count(const char *path, const char *table) {
    sqlite3 *db=NULL;assert(sqlite3_open_v2(path,&db,SQLITE_OPEN_READONLY,NULL)==SQLITE_OK);
    char sql[200];snprintf(sql,sizeof(sql),"SELECT count(*) FROM %s;",table);
    sqlite3_stmt *s=NULL;assert(sqlite3_prepare_v2(db,sql,-1,&s,NULL)==SQLITE_OK);
    assert(sqlite3_step(s)==SQLITE_ROW);int64_t n=sqlite3_column_int64(s,0);
    sqlite3_finalize(s);sqlite3_close(db);return n;
}
static void raw_sql(const char *path, const char *sql) {
    sqlite3 *db=NULL;assert(sqlite3_open_v2(path,&db,SQLITE_OPEN_READWRITE,NULL)==SQLITE_OK);
    int rc=sqlite3_exec(db,sql,NULL,NULL,NULL);
    if(rc!=SQLITE_OK){fprintf(stderr,"SQL failed rc=%d: %s\n",rc,sqlite3_errmsg(db));abort();}
    assert(sqlite3_close(db)==SQLITE_OK);
}
static atto_event_command command(const char *id,uint64_t epoch,uint64_t ver,
                                 const char *op,const char *bytes,
                                 const atto_event_blob *events,size_t count,
                                 const atto_event_blob *outbox,size_t outcount) {
    atto_event_command c={0};
    c.actor_id=id;c.state_slot="main";c.epoch=epoch;c.expected_version=ver;
    c.operation_id=op;c.command_bytes=bytes;c.command_length=strlen(bytes);
    c.events=events;c.event_count=count;c.outbox=outbox;c.outbox_count=outcount;
    return c;
}
static void test_atomicity_dedupe(const char *path) {
    atto_event_journal *j=NULL;
    expect(atto_event_journal_open(":memory:",&j),ATTO_EVENT_INVALID,"reject memory");
    expect(atto_event_journal_open(path,&j),ATTO_EVENT_OK,"journal open");
    uint64_t epoch=0;expect(atto_event_journal_activate(j,"tenant/acc:7",&epoch),ATTO_EVENT_OK,"activate");
    assert(epoch==1);
    const char first[]="+20",second[]="-5",outbound[]="send-receipt";
    atto_event_blob events[]={{first,sizeof(first)-1},{second,sizeof(second)-1}};
    atto_event_blob outbox[]={{outbound,sizeof(outbound)-1}};
    atto_event_command c=command("tenant/acc:7",epoch,0,"op-1","deposit-20-withdraw-5",events,2,outbox,1);
    atto_event_receipt r={0};
    expect(atto_event_journal_append(j,&c,&r),ATTO_EVENT_OK,"batch append");
    assert(r.from_version==0&&r.to_version==2&&r.replayed==0);
    assert(sql_count(path,"atto_event_stream_events")==2);
    assert(sql_count(path,"atto_event_stream_operations")==1);
    assert(sql_count(path,"atto_event_stream_outbox")==1);
    atto_event_record *found=NULL;size_t count=0;uint64_t head=0;
    expect(atto_event_journal_read(j,c.actor_id,c.state_slot,0,64,&found,&count,&head),0,"read");
    assert(head==2&&count==2&&found[0].sequence==1&&found[1].sequence==2);
    assert(found[0].length==3&&memcmp(found[0].bytes,"+20",3)==0);
    assert(found[1].length==2&&memcmp(found[1].bytes,"-5",2)==0);
    atto_event_journal_free_records(found,count);
    expect(atto_event_journal_append(j,&c,&r),ATTO_EVENT_OK,"dedupe original");
    assert(r.replayed==1&&r.to_version==2);
    assert(sql_count(path,"atto_event_stream_events")==2);
    assert(sql_count(path,"atto_event_stream_outbox")==1);
    expect(atto_event_journal_resolve(j,c.actor_id,c.state_slot,c.operation_id,
            c.command_bytes,c.command_length,&r),ATTO_EVENT_OK,"reconcile receipt");
    assert(r.replayed==1&&r.from_version==0&&r.to_version==2);
    expect(atto_event_journal_resolve(j,c.actor_id,c.state_slot,"missing",
            c.command_bytes,c.command_length,&r),ATTO_EVENT_ABSENT,"missing receipt");
    expect(atto_event_journal_resolve(j,c.actor_id,c.state_slot,c.operation_id,
            "different",9,&r),ATTO_EVENT_OPERATION_CONFLICT,"mismatch receipt");
    c.command_bytes="different";c.command_length=9;
    expect(atto_event_journal_append(j,&c,&r),ATTO_EVENT_OPERATION_CONFLICT,"mismatched operation ID");
    c.command_bytes="deposit-20-withdraw-5";c.command_length=strlen(c.command_bytes);
    c.expected_version=1;
    expect(atto_event_journal_append(j,&c,&r),ATTO_EVENT_OPERATION_CONFLICT,"bind expected version into dedupe");
    c.operation_id="op-2";
    expect(atto_event_journal_append(j,&c,&r),ATTO_EVENT_VERSION_CONFLICT,"stale stream head");
    c.expected_version=2;
    atto_actor_db *checkpoint=NULL;
    expect(atto_actor_db_open(path,&checkpoint),0,"shared store open");
    uint64_t new_epoch=0;
    expect(atto_actor_activate(checkpoint,c.actor_id,&new_epoch),0,"new checkpoint epoch");
    assert(new_epoch==epoch+1);
    c.epoch=epoch;c.operation_id="op-1";c.expected_version=0;
    expect(atto_event_journal_append(j,&c,&r),ATTO_EVENT_FENCED,"fence even original id");
    c.epoch=new_epoch;c.operation_id="op-2";c.expected_version=2;
    expect(atto_event_journal_append(j,&c,&r),ATTO_EVENT_OK,"new epoch append");
    assert(r.to_version==4);
    atto_actor_db_close(checkpoint);
    atto_event_journal_close(j);
}
static void test_rollback_and_corruption(const char *path) {
    atto_event_journal *j=NULL;expect(atto_event_journal_open(path,&j),0,"open journal");
    uint64_t epoch=0;expect(atto_event_journal_activate(j,"tenant/rollback",&epoch),0,"activate rollback stream");
    raw_sql(path,"CREATE TRIGGER reject_outbox BEFORE INSERT ON atto_event_stream_outbox "
                 "WHEN hex(NEW.payload) = '4641494C' BEGIN SELECT RAISE(ABORT,'injected outbox failure'); END;");
    atto_event_blob events[]={{"+1",2}};atto_event_blob outbox[]={{"FAIL",4}};
    atto_event_command c=command("tenant/rollback",epoch,0,"opbad","canonical",events,1,outbox,1);
    atto_event_receipt receipt={0};
    expect(atto_event_journal_append(j,&c,&receipt),ATTO_EVENT_IO,"outbox abort rolls back all");
    assert(sql_count(path,"atto_event_stream_outbox")==2); /* prior 1 + new epoch 1 from prior test */
    /* The failed actor stream doesn't even have a head; nothing committed. */
    atto_event_record *rows=NULL;size_t len=0;uint64_t head=0;
    expect(atto_event_journal_read(j,c.actor_id,c.state_slot,0,10,&rows,&len,&head),ATTO_EVENT_ABSENT,"rollback no head");
    assert(rows==NULL);
    expect(atto_event_journal_resolve(j,c.actor_id,c.state_slot,"opbad","canonical",9,&receipt),ATTO_EVENT_ABSENT,"rollback no dedupe");
    raw_sql(path,"DROP TRIGGER reject_outbox;");
    c.outbox_count=0;
    expect(atto_event_journal_append(j,&c,&receipt),0,"commit after failure");
    raw_sql(path,"UPDATE atto_event_stream_events SET payload = 'tampered' "
                 "WHERE actor_id = 'tenant/rollback' AND version=1;");
    expect(atto_event_journal_read(j,c.actor_id,c.state_slot,0,10,&rows,&len,&head),ATTO_EVENT_CORRUPT,"payload hash tamper");
    raw_sql(path,"DELETE FROM atto_event_stream_events WHERE actor_id = 'tenant/rollback' AND version=1;");
    expect(atto_event_journal_read(j,c.actor_id,c.state_slot,0,10,&rows,&len,&head),ATTO_EVENT_CORRUPT,"missing sequence");
    atto_event_journal_close(j);
}
static void signal_kill(pid_t pid,int from_child) {
    char b=0;assert(read(from_child,&b,1)==1&&b=='R');
    assert(kill(pid,SIGKILL)==0);
    int status=0;assert(waitpid(pid,&status,0)==pid&&WIFSIGNALED(status)&&WTERMSIG(status)==SIGKILL);
    close(from_child);
}
static void kill_after_commit(const char *path) {
    int pipes[2];assert(pipe(pipes)==0);
    pid_t pid=fork();assert(pid>=0);
    if(pid==0){
        close(pipes[0]);
        atto_event_journal *j=NULL;assert(atto_event_journal_open(path,&j)==0);
        uint64_t epoch=0;assert(atto_event_journal_activate(j,"tenant/crash",&epoch)==0);
        for(int i=0;i<100;i++){
            char op[32],cmd[32];snprintf(op,sizeof(op),"deposit_%d",i);
            snprintf(cmd,sizeof(cmd),"deposit 1 nr %d",i);
            atto_event_blob delta={"+1",2};
            atto_event_command c=command("tenant/crash",epoch,(uint64_t)i,op,cmd,&delta,1,NULL,0);
            atto_event_receipt receipt={0};assert(atto_event_journal_append(j,&c,&receipt)==0);
            assert(receipt.to_version==(uint64_t)i+1);
        }
        assert(write(pipes[1],"R",1)==1);
        for(;;)pause();
    }
    close(pipes[1]);signal_kill(pid,pipes[0]);
    atto_event_journal *j=NULL;expect(atto_event_journal_open(path,&j),0,"after kill open");
    uint64_t epoch=0;expect(atto_event_journal_activate(j,"tenant/crash",&epoch),0,"fenced revival");
    assert(epoch==2);
    int64_t balance=0;uint64_t cursor=0,head=0;
    for(;;){
        atto_event_record *rows=NULL;size_t count=0;
        expect(atto_event_journal_read(j,"tenant/crash","main",cursor,17,&rows,&count,&head),0,"replay page");
        for(size_t i=0;i<count;i++){
            assert(rows[i].sequence==cursor+i+1);
            assert(rows[i].length==2&&!memcmp(rows[i].bytes,"+1",2));
            balance+=1;
        }
        cursor+=count;atto_event_journal_free_records(rows,count);
        if(cursor==head)break;
        assert(count>0);
    }
    assert(balance==100&&head==100);
    atto_event_blob delta={"+1",2};
    atto_event_command original=command("tenant/crash",epoch,99,"deposit_99","deposit 1 nr 99",&delta,1,NULL,0);
    atto_event_receipt receipt={0};
    expect(atto_event_journal_append(j,&original,&receipt),0,"retry after lost ACK");
    assert(receipt.replayed==1&&receipt.to_version==100);
    atto_event_command next=command("tenant/crash",epoch,100,"deposit_100","deposit 1 nr 100",&delta,1,NULL,0);
    expect(atto_event_journal_append(j,&next,&receipt),0,"next command after recovery");
    assert(!receipt.replayed&&receipt.to_version==101);
    atto_event_journal_close(j);
}
static void kill_before_commit(const char *path) {
    int pipes[2];assert(pipe(pipes)==0);
    pid_t pid=fork();assert(pid>=0);
    if(pid==0){
        close(pipes[0]);
        sqlite3 *db=NULL;assert(sqlite3_open_v2(path,&db,SQLITE_OPEN_READWRITE,NULL)==SQLITE_OK);
        assert(sqlite3_exec(db,"BEGIN IMMEDIATE;",NULL,NULL,NULL)==SQLITE_OK);
        assert(sqlite3_exec(db,"UPDATE atto_event_stream_heads SET head_version=102 "
                   "WHERE actor_id='tenant/crash';",NULL,NULL,NULL)==SQLITE_OK);
        assert(sqlite3_exec(db,"INSERT INTO atto_event_stream_events(actor_id,state_slot,version,payload,payload_sha256) "
                   "SELECT actor_id,state_slot,102,payload,payload_sha256 "
                   "FROM atto_event_stream_events WHERE actor_id='tenant/crash' AND version=101;",NULL,NULL,NULL)==SQLITE_OK);
        assert(write(pipes[1],"R",1)==1);
        for(;;)pause();
    }
    close(pipes[1]);signal_kill(pid,pipes[0]);
    atto_event_journal *j=NULL;assert(atto_event_journal_open(path,&j)==0);
    atto_event_record *rows=NULL;size_t count=0;uint64_t head=0;
    expect(atto_event_journal_read(j,"tenant/crash","main",100,10,&rows,&count,&head),0,"no torn commit");
    assert(head==101&&count==1&&rows[0].sequence==101);
    atto_event_journal_free_records(rows,count);atto_event_journal_close(j);
}
int main(void) {
    char dir[]="/tmp/atto-event-journal-XXXXXX";assert(mkdtemp(dir));
    char path[512];snprintf(path,sizeof(path),"%s/journal.sqlite",dir);
    test_atomicity_dedupe(path);
    test_rollback_and_corruption(path);
    kill_after_commit(path);
    kill_before_commit(path);
    char wal[532],shm[532];snprintf(wal,sizeof(wal),"%s-wal",path);snprintf(shm,sizeof(shm),"%s-shm",path);
    unlink(wal);unlink(shm);unlink(path);rmdir(dir);
    puts("PASS native SQLite event journal: atomic batch/outbox/dedupe, epoch fence, SHA-256, 100 replay, SIGKILL before/after commit");
    return 0;
}
