#define _POSIX_C_SOURCE 200809L
#include "atto_actor_sqlite_v1.h"
#include <assert.h>
#include <inttypes.h>
#include <signal.h>
#include <sqlite3.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>

/* Host actor simulation, not compiled AttoLang. One message -> one SQLite commit. */
static const uint64_t SCHEMA=UINT64_C(0xe435bba927ce0112);
static void put64(uint8_t *p,uint64_t v){for(int i=0;i<8;i++)p[i]=(uint8_t)(v>>(8*i));}
static uint64_t get64(const uint8_t*p){uint64_t v=0;for(int i=0;i<8;i++)v|=(uint64_t)p[i]<<(8*i);return v;}
static uint64_t hash64(const uint8_t*p,size_t n){uint64_t h=UINT64_C(0xcbf29ce484222325);for(size_t i=0;i<n;i++){h^=p[i];h*=UINT64_C(0x100000001b3);}return h;}
static void pack(int64_t value,uint8_t b[36]){
    memcpy(b,"ATTOST01",8);b[8]=1;put64(b+9,SCHEMA);b[17]=1;b[18]=0;b[19]=1;
    put64(b+20,(uint64_t)value);put64(b+28,hash64(b,28));
}
static int64_t revive(atto_actor_db*db,uint64_t*seq){
    void*p=NULL;size_t n=0;int rc=atto_actor_load(db,"counter:unit",SCHEMA,&p,&n,seq);
    if(rc==ATTO_ACTOR_STORE_ABSENT){*seq=0;return 0;}
    if(rc!=ATTO_ACTOR_STORE_OK || n!=36){fprintf(stderr,"cannot revive actor, rc=%d\n",rc);exit(2);}
    int64_t result=(int64_t)get64((uint8_t*)p+20);atto_actor_free(p);return result;
}
static void handle_increment(atto_actor_db*db,uint64_t epoch,uint64_t*seq,int64_t*count){
    uint8_t data[36]; pack(*count+1,data);
    uint64_t next=0;
    int rc=atto_actor_commit(db,"counter:unit",epoch,*seq,SCHEMA,data,sizeof(data),&next);
    if(rc!=ATTO_ACTOR_STORE_OK || next!=*seq+1){fprintf(stderr,"increment commit rc=%d\n",rc);exit(2);}
    *seq=next;*count+=1;
}
int main(void){
    char directory[]="/tmp/atto-counter-revival-XXXXXX";
    assert(mkdtemp(directory));
    char path[512];snprintf(path,sizeof(path),"%s/actors.sqlite",directory);
    int pipefd[2];assert(pipe(pipefd)==0);
    pid_t child=fork();assert(child>=0);
    if(child==0){
        close(pipefd[0]);
        atto_actor_db*db=NULL;
        assert(atto_actor_db_open(path,&db)==ATTO_ACTOR_STORE_OK);
        uint64_t epoch=0,seq=0;
        assert(atto_actor_activate(db,"counter:unit",&epoch)==ATTO_ACTOR_STORE_OK);
        int64_t count=revive(db,&seq);
        assert(count==0);
        for(int i=0;i<100;i++)handle_increment(db,epoch,&seq,&count);
        assert(count==100 && seq==100);
        assert(write(pipefd[1],"R",1)==1);
        for(;;)pause();
    }
    close(pipefd[1]);char signal_byte=0;
    assert(read(pipefd[0],&signal_byte,1)==1 && signal_byte=='R');
    assert(kill(child,SIGKILL)==0);
    int status=0;assert(waitpid(child,&status,0)==child && WIFSIGNALED(status));
    close(pipefd[0]);
    atto_actor_db*db=NULL;assert(atto_actor_db_open(path,&db)==ATTO_ACTOR_STORE_OK);
    uint64_t epoch=0,seq=0;assert(atto_actor_activate(db,"counter:unit",&epoch)==ATTO_ACTOR_STORE_OK);
    assert(epoch==2);
    int64_t count=revive(db,&seq);
    assert(count==100 && seq==100);
    handle_increment(db,epoch,&seq,&count);
    assert(count==101 && seq==101);
    atto_actor_db_close(db);
    char wal[530],shm[530];snprintf(wal,sizeof(wal),"%s-wal",path);snprintf(shm,sizeof(shm),"%s-shm",path);
    unlink(wal);unlink(shm);unlink(path);rmdir(directory);
    puts("PASS native host counter simulation: 100 committed increments, SIGKILL, revive 100, increment to 101");
    return 0;
}
