#define _POSIX_C_SOURCE 200809L
#include "atto_actor_sqlite_v1.h"
#include <assert.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>
#include <sqlite3.h>
#include <fcntl.h>

static void le64(unsigned char *p,uint64_t v){for(int i=0;i<8;i++)p[i]=(unsigned char)(v>>(i*8));}
static uint64_t fnv(const unsigned char *p,size_t len){uint64_t h=UINT64_C(0xcbf29ce484222325);for(size_t i=0;i<len;i++){h^=p[i];h*=UINT64_C(0x100000001b3);}return h;}
/* One int scalar matching the Rust ATTOST01 serialization byte-for-byte. */
static void snapshot(uint64_t schema,int64_t value,unsigned char out[36]){
    memcpy(out,"ATTOST01",8);out[8]=1;le64(out+9,schema);out[17]=1;out[18]=0;out[19]=1;
    le64(out+20,(uint64_t)value);le64(out+28,fnv(out,28));
}
static const uint64_t SCHEMA=UINT64_C(0xe435bba927ce0112);
static void must(int actual,int want,const char*what){if(actual!=want){fprintf(stderr,"FAIL %s: got %d expected %d\n",what,actual,want);exit(2);}}
static void recover(atto_actor_db *db,const char *id,int64_t expected,uint64_t sequence){
    void *p=NULL;size_t size=0;uint64_t s=0;
    must(atto_actor_load(db,id,SCHEMA,&p,&size,&s),0,"load");
    assert(size==36 && s==sequence);
    const unsigned char *b=p;
    uint64_t u=0;for(int i=0;i<8;i++)u|=(uint64_t)b[20+i]<<(8*i);
    assert(u==(uint64_t)expected);
    atto_actor_free(p);
}
static void await_kill(pid_t child,int fd){
    char ch=0;
    ssize_t n=read(fd,&ch,1);
    if(n!=1 || ch!='R'){fprintf(stderr,"child handshake failed: %zd\n",n);exit(2);}
    must(kill(child,SIGKILL),0,"kill child");
    int status=0;
    assert(waitpid(child,&status,0)==child && WIFSIGNALED(status) && WTERMSIG(status)==SIGKILL);
    close(fd);
}
static void test_kill_after_commit(const char *path,uint64_t initial_seq){
    int fds[2];assert(pipe(fds)==0);
    pid_t pid=fork();assert(pid>=0);
    if(pid==0){
        close(fds[0]);
        atto_actor_db *db=NULL;must(atto_actor_db_open(path,&db),0,"child open");
        uint64_t epoch=0,next=0;must(atto_actor_activate(db,"actor:1",&epoch),0,"child activate");
        unsigned char b[36];snapshot(SCHEMA,501,b);
        must(atto_actor_commit(db,"actor:1",epoch,initial_seq,SCHEMA,b,sizeof(b),&next),0,"child commit");
        assert(next==initial_seq+1);
        assert(write(fds[1],"R",1)==1);
        for(;;)pause();
    }
    close(fds[1]);await_kill(pid,fds[0]);
    atto_actor_db *db=NULL;must(atto_actor_db_open(path,&db),0,"reopen after kill");
    recover(db,"actor:1",501,initial_seq+1);
    atto_actor_db_close(db);
}
static void test_kill_before_commit(const char *path,uint64_t sequence){
    int fds[2];assert(pipe(fds)==0);
    pid_t pid=fork();assert(pid>=0);
    if(pid==0){
        close(fds[0]);
        sqlite3 *db=NULL;
        assert(sqlite3_open_v2(path,&db,SQLITE_OPEN_READWRITE|SQLITE_OPEN_FULLMUTEX,NULL)==SQLITE_OK);
        assert(sqlite3_exec(db,"BEGIN IMMEDIATE;",NULL,NULL,NULL)==SQLITE_OK);
        /* Test a torn/uncommitted row modification immediately prior to kill. */
        unsigned char b[36];snapshot(SCHEMA,900,b);
        sqlite3_stmt *st=NULL;
        assert(sqlite3_prepare_v2(db,"UPDATE atto_actor_snapshots SET snapshot=?1 WHERE actor_id='actor:1'",-1,&st,NULL)==SQLITE_OK);
        assert(sqlite3_bind_blob(st,1,b,sizeof(b),SQLITE_TRANSIENT)==SQLITE_OK);
        assert(sqlite3_step(st)==SQLITE_DONE);
        sqlite3_finalize(st);
        assert(write(fds[1],"R",1)==1);
        for(;;)pause();
    }
    close(fds[1]);await_kill(pid,fds[0]);
    atto_actor_db *db=NULL;must(atto_actor_db_open(path,&db),0,"reopen rollback");
    recover(db,"actor:1",501,sequence);
    atto_actor_db_close(db);
}
int main(void){
    char dir[]="/tmp/atto-actor-sqlite-XXXXXX";
    assert(mkdtemp(dir)!=NULL);
    char path[512];snprintf(path,sizeof(path),"%s/actors.sqlite",dir);
    atto_actor_db *db=NULL;
    must(atto_actor_db_open(":memory:",&db),ATTO_ACTOR_STORE_INVALID,"memory disallowed");
    must(atto_actor_db_open(path,&db),0,"create database");
    uint64_t epoch=0,seq=0;
    must(atto_actor_activate(db,"actor:1",&epoch),0,"activate"); assert(epoch==1);
    void *missing=NULL;size_t missing_len=99;uint64_t missing_seq=99;
    must(atto_actor_load(db,"actor:1",SCHEMA,&missing,&missing_len,&missing_seq),ATTO_ACTOR_STORE_ABSENT,"missing");
    assert(missing==NULL && missing_len==0 && missing_seq==0);
    unsigned char one[36];snapshot(SCHEMA,42,one);
    must(atto_actor_validate_snapshot(one,sizeof(one),SCHEMA),0,"snapshot valid");
    must(atto_actor_commit(db,"actor:1",epoch,0,SCHEMA,one,sizeof(one),&seq),0,"first commit"); assert(seq==1);
    must(atto_actor_commit(db,"actor:1",epoch,0,SCHEMA,one,sizeof(one),&seq),ATTO_ACTOR_STORE_STALE_SEQUENCE,"sequence CAS");
    recover(db,"actor:1",42,1);
    must(atto_actor_load(db,"actor:1",SCHEMA+1,&missing,&missing_len,&missing_seq),ATTO_ACTOR_STORE_SCHEMA,"load other schema");
    unsigned char bad[36];memcpy(bad,one,sizeof(bad));bad[35]^=1;
    must(atto_actor_commit(db,"actor:1",epoch,1,SCHEMA,bad,sizeof(bad),&seq),ATTO_ACTOR_STORE_CORRUPT,"invalid checksum");
    must(atto_actor_commit(db,"actor:1",epoch,1,SCHEMA+1,one,sizeof(one),&seq),ATTO_ACTOR_STORE_SCHEMA,"invalid schema");
    atto_actor_db *other=NULL;must(atto_actor_db_open(path,&other),0,"second instance open");
    uint64_t new_epoch=0;must(atto_actor_activate(other,"actor:1",&new_epoch),0,"fence first");assert(new_epoch==epoch+1);
    must(atto_actor_commit(db,"actor:1",epoch,1,SCHEMA,one,sizeof(one),&seq),ATTO_ACTOR_STORE_STALE_EPOCH,"old epoch fenced");
    unsigned char two[36];snapshot(SCHEMA,100,two);
    must(atto_actor_commit(other,"actor:1",new_epoch,1,SCHEMA,two,sizeof(two),&seq),0,"new epoch commit");assert(seq==2);
    atto_actor_db_close(other);atto_actor_db_close(db);
    test_kill_after_commit(path,2);
    test_kill_before_commit(path,3);
    must(atto_actor_db_open(path,&db),0,"final reopen");
    recover(db,"actor:1",501,3);
    must(atto_actor_activate(db,"actor:1",&new_epoch),0,"new final epoch");assert(new_epoch>=4);
    must(atto_actor_commit(db,"actor:1",new_epoch,2,SCHEMA,two,sizeof(two),&seq),ATTO_ACTOR_STORE_STALE_SEQUENCE,"reject replay");
    must(atto_actor_commit(db,"actor:1",new_epoch,3,SCHEMA,two,sizeof(two),&seq),0,"final commit");assert(seq==4);
    atto_actor_db_close(db);
    /* Delete test database and journal artifacts from trusted tempdir. */
    char wal[532],shm[532];snprintf(wal,sizeof(wal),"%s-wal",path);snprintf(shm,sizeof(shm),"%s-shm",path);
    unlink(wal);unlink(shm);unlink(path);rmdir(dir);
    puts("PASS native sqlite actor state: epoch CAS, corruption, SIGKILL after/before COMMIT, reload");
    return 0;
}
