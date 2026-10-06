/* Direct userspace NFS backend. nfs_mount is libnfs's session setup,
 * NOT a Linux kernel mount nor a GVfs mount. Read-only operations only. */
#define _DEFAULT_SOURCE

#include <stddef.h>
#include <sys/time.h>
#include <nfsc/libnfs.h>
#include <nfsc/libnfs-raw.h>
#include <nfsc/libnfs-raw-mount.h>
#include <sys/stat.h>
#include <fcntl.h>
#include <errno.h>
#include <stdlib.h>
#include <stdio.h>
#include <string.h>
#include <stdint.h>
#include <limits.h>
#include <pthread.h>
#include <time.h>
#include <netdb.h>
#include <unistd.h>

/* Cancellation belongs to this worker, never to foreground viewer reads. */
typedef int (*pic_cancel_cb)(void *);
static _Thread_local pic_cancel_cb scan_cancel = NULL;
static _Thread_local void *scan_cancel_context = NULL;
void pic_nfs_set_scan_cancel(pic_cancel_cb callback, void *context) {
    scan_cancel = callback; scan_cancel_context = context;
}
static int scan_cancelled(void) {
    return scan_cancel && scan_cancel(scan_cancel_context);
}
static int cancelled_error(char *error, size_t cap) {
    if (!scan_cancelled()) return 0;
    snprintf(error, cap, "NFS scan cancelled"); return 1;
}
typedef int (*pic_entry_cb)(void *, const char *, unsigned int);
static int trace_enabled(void) {
    const char *value = getenv("PICASA_TRACE");
    return value && *value;
}
static uint64_t monotonic_ms(void) {
    struct timespec now;
    clock_gettime(CLOCK_MONOTONIC, &now);
    return (uint64_t)now.tv_sec * 1000 + now.tv_nsec / 1000000;
}
static void trace_stage(const char *stage, uint64_t started, const char *host,
                        const char *export_path, const char *relative,
                        const char *detail) {
    if (!trace_enabled()) return;
    fprintf(stderr, "PIC_NFS stage=%s elapsed_ms=%llu tid=%lu host=%s export=%s relative=%s%s%s\n",
            stage, (unsigned long long)(monotonic_ms() - started),
            (unsigned long)pthread_self(), host ? host : "-",
            export_path ? export_path : "-", relative ? relative : "-",
            detail ? " " : "", detail ? detail : "");
}
static void trace_hostname_resolution(const char *host) {
    if (!trace_enabled()) return;
    uint64_t started = monotonic_ms();
    struct addrinfo hints = {0}, *results = NULL;
    hints.ai_socktype = SOCK_STREAM;
    int status = getaddrinfo(host, NULL, &hints, &results);
    int count = 0;
    for (struct addrinfo *entry = results; entry; entry = entry->ai_next) count++;
    char detail[64];
    snprintf(detail, sizeof detail, "status=%d addresses=%d", status, count);
    trace_stage("hostname_resolution", started, host, NULL, NULL, detail);
    if (results) freeaddrinfo(results);
}
static void err(char *dst, size_t cap, const char *op, struct nfs_context *nfs) {
    const char *detail = nfs ? nfs_get_error(nfs) : NULL;
    snprintf(dst, cap, "%s: %s", op,
             detail && *detail ? detail : "libnfs returned no error details");
}
/* libnfs defaults to v3 and can silently retry v4, obscuring the first
 * failure. Make each attempt explicit and preserve BOTH diagnostics. This
 * session stays inside this process; it never creates a GVfs/kernel mount. */
static struct nfs_context *open_session(const char *host, const char *export_path,
                                        char *error, size_t cap) {
    char attempts[768] = {0};
    for (int version = 3; version <= 4; version++) {
        if (cancelled_error(error,cap)) return NULL;
        uint64_t context_started = monotonic_ms();
        struct nfs_context *nfs = nfs_init_context();
        trace_stage("context_creation", context_started, host, export_path, NULL,
                    nfs ? "outcome=ok" : "outcome=error");
        if (!nfs) {
            snprintf(error, cap, "nfs_init_context failed for %s", host);
            return NULL;
        }
        int selected = nfs_set_version(nfs, version);
        if (selected != 0) {
            snprintf(error, cap, "nfs_set_version(%d) failed: status=%d", version, selected);
            nfs_destroy_context(nfs);
            return NULL;
        }
        /* Disable endless reconnection during the initial connection probe. */
        nfs_set_autoreconnect(nfs, 0);
        nfs_set_timeout(nfs, 5000);
        char session_detail[128];
        snprintf(session_detail, sizeof session_detail,
                 "version=%d uid=%ld gid=%ld source_port=libnfs_default",
                 version, (long)geteuid(), (long)getegid());
        trace_stage("session_configuration", context_started, host, export_path, NULL,
                    session_detail);
        uint64_t connect_started = monotonic_ms();
        if (trace_enabled()) fprintf(stderr, "PIC_NFS_CONNECT create_start host=%s export=%s version=%d tid=%lu\n",
                                     host, export_path, version, (unsigned long)pthread_self());
        int status = nfs_mount(nfs, host, export_path);
        char connect_detail[64];
        snprintf(connect_detail, sizeof connect_detail, "version=%d outcome=%s status=%d",
                 version, status == 0 ? "ok" : "error", status);
        trace_stage("connection_establishment", connect_started, host, export_path, NULL,
                    connect_detail);
        if (status == 0) {
            if (trace_enabled()) fprintf(stderr, "PIC_NFS_CONNECT create_ok host=%s export=%s version=%d\n",
                                         host, export_path, version);
            return nfs;
        }
        const char *detail = nfs_get_error(nfs);
        char line[384];
        snprintf(line, sizeof line, "%sv%d status=%d (%s)%s%s",
                 attempts[0] ? "; " : "", version, status,
                 status < 0 && -status < 4096 ? strerror(-status) : "unknown status",
                 detail && *detail ? ": " : "", detail && *detail ? detail : "");
        size_t used = strlen(attempts);
        if (used < sizeof(attempts) - 1)
            snprintf(attempts + used, sizeof(attempts) - used, "%s", line);
        if (trace_enabled()) fprintf(stderr, "PIC_NFS_CONNECT create_failed host=%s export=%s %s\n",
                                     host, export_path, line);
        nfs_destroy_context(nfs);
    }
    snprintf(error, cap,
             "NFS session failed host=%s export=%s; %.250s; if the NAS requires reserved client source ports, add the server-side 'insecure' export option for this read-only export",
             host, export_path, attempts);
    return NULL;
}
int pic_nfs_exports(const char *host, pic_entry_cb cb, void *ctx,
                    char *error, size_t cap) {
    trace_hostname_resolution(host);
    uint64_t discovery_started = monotonic_ms();
    struct exportnode *list = mount_getexports(host);
    if (!list) {
        trace_stage("export_discovery", discovery_started, host, NULL, NULL, "outcome=error");
        snprintf(error, cap, "NFS export discovery failed for %s (check rpcbind/mountd and NFSv3)", host); return -1;
    }
    int total=0;
    for (struct exportnode *e=list;e;e=e->ex_next) {
        if (e->ex_dir && cb(ctx,e->ex_dir,7) != 0) break;
        total++;
    }
    mount_free_export_list(list);
    char detail[48];
    snprintf(detail, sizeof detail, "outcome=ok exports=%d", total);
    trace_stage("export_discovery", discovery_started, host, NULL, NULL, detail);
    return total;
}
int pic_nfs_list(const char *host, const char *export_path, const char *relative,
                 pic_entry_cb cb, void *context, char *error, size_t cap) {
    struct nfs_context *nfs=open_session(host,export_path,error,cap);
    if (!nfs) return -1;
    struct nfsdir *dir=NULL;
    uint64_t open_started = monotonic_ms();
    if (nfs_opendir(nfs,relative,&dir)!=0) {
        err(error,cap,"nfs_opendir",nfs);
        trace_stage("directory_open", open_started, host, export_path, relative, "outcome=error");
        if (strstr(error,"NFS4ERR_PERM") || strstr(error,"Permission denied")) {
            char detail[512];
            snprintf(detail,sizeof detail,"%s",error);
            snprintf(error,cap,"%.340s; server denied directory access (check export path, client UID/GID and directory execute/read permissions)",detail);
        }
        nfs_destroy_context(nfs);return -1;
    }
    trace_stage("directory_open", open_started, host, export_path, relative, "outcome=ok");
    int count=0;
    struct nfsdirent *entry;
    while ((entry=nfs_readdir(nfs,dir))) {
        if (!entry->name || strcmp(entry->name,".")==0 || strcmp(entry->name,"..")==0) continue;
        unsigned int kind=(entry->mode & S_IFMT)==S_IFDIR ? 7 :
                           (entry->mode & S_IFMT)==S_IFREG ? 8 : 0;
        if (kind && cb(context,entry->name,kind)!=0) break;
        count++;
    }
    nfs_closedir(nfs,dir);
    char detail[64];
    snprintf(detail,sizeof detail,"outcome=ok entries=%d",count);
    trace_stage("directory_list_complete", open_started, host, export_path, relative, detail);
    nfs_destroy_context(nfs);
    return count;
}

/* The discovery worker owns this session. Unlike picker listings, it streams
 * attributes from READDIRPLUS and reuses its connection across directories. */
static _Thread_local struct nfs_context *scan_session = NULL;
static _Thread_local char scan_host[256] = {0};
static _Thread_local char scan_export[4096] = {0};
void pic_nfs_scan_close(void) {
    if (scan_session) nfs_destroy_context(scan_session);
    scan_session = NULL; scan_host[0] = 0; scan_export[0] = 0;
}
typedef int (*pic_scan_cb)(void *, const char *, unsigned int, uint64_t, int64_t, int);
int pic_nfs_scan(const char *host, const char *export_path, const char *relative,
                 pic_scan_cb cb, void *context, char *error, size_t cap) {
    if (!scan_session || strcmp(scan_host, host) || strcmp(scan_export, export_path)) {
        pic_nfs_scan_close();
        scan_session = open_session(host, export_path, error, cap);
        if (!scan_session) return -1;
        snprintf(scan_host, sizeof scan_host, "%s", host);
        snprintf(scan_export, sizeof scan_export, "%s", export_path);
    }
    struct nfsdir *dir = NULL;
    if (nfs_opendir(scan_session, relative, &dir) != 0) {
        err(error, cap, "nfs_opendir", scan_session);
        pic_nfs_scan_close(); return -1;
    }
    struct nfsdirent *entry;
    while ((entry = nfs_readdir(scan_session, dir))) {
        if (!entry->name || !strcmp(entry->name, ".") || !strcmp(entry->name, "..")) continue;
        unsigned int kind = (entry->mode & S_IFMT) == S_IFDIR ? 7 :
                            (entry->mode & S_IFMT) == S_IFREG ? 8 : 0;
        if (kind && cb(context, entry->name, kind, entry->size,
                       (int64_t)entry->mtime.tv_sec, entry->mode != 0) != 0) break;
    }
    nfs_closedir(scan_session, dir);
    return 0;
}
/* A libnfs context must never be used concurrently. Viewer reads run on
 * short-lived Rust threads, so thread-local storage caused every image read to
 * create a fresh session. Keep one process-local session and serialize its
 * complete operation; failed NFS operations invalidate it. The context is NOT
 * a GVfs or Linux kernel mount. */
static pthread_mutex_t read_session_lock = PTHREAD_MUTEX_INITIALIZER;
static int lock_read_session(char *error, size_t cap) {
    if (!scan_cancel) { pthread_mutex_lock(&read_session_lock); return 0; }
    while (!cancelled_error(error, cap)) {
        int result = pthread_mutex_trylock(&read_session_lock);
        if (result == 0) {
            if (!cancelled_error(error, cap)) return 0;
            pthread_mutex_unlock(&read_session_lock); return -1;
        }
        if (result != EBUSY) { snprintf(error, cap, "NFS read lock: %s", strerror(result)); return -1; }
        struct timespec delay = {0, 10000000};
        nanosleep(&delay, NULL);
    }
    return -1;
}

static struct nfs_context *read_session=NULL;
static char read_host[256]={0};
static char read_export[4096]={0};
static void invalidate_read_session(void) {
    if (read_session) nfs_destroy_context(read_session);
    read_session=NULL;
    read_host[0]=0;
    read_export[0]=0;
}
static struct nfs_context *get_read_session(const char *host, const char *export_path,
                                            char *error, size_t cap) {
    if (!read_session || strcmp(read_host,host) || strcmp(read_export,export_path)) {
        invalidate_read_session();
        read_session=open_session(host,export_path,error,cap);
        if (!read_session) return NULL;
        snprintf(read_host,sizeof read_host,"%s",host);
        snprintf(read_export,sizeof read_export,"%s",export_path);
    } else {
        if (trace_enabled()) fprintf(stderr,"PIC_NFS_CONNECT reuse host=%s export=%s\n",host,export_path);
    }
    return read_session;
}
/* Free the opened handle through the public API even on Stop. Restore the
 * normal timeout while still holding the lock, before reuse or disposal. */
static int close_read_handle(struct nfs_context *nfs, struct nfsfh *fh) {
    int canceled=scan_cancelled();
    if (canceled) nfs_set_timeout(nfs, 250);
    int result=nfs_close(nfs, fh);
    if (canceled) nfs_set_timeout(nfs, 5000);
    return result;
}
int pic_nfs_read(const char *host, const char *export_path, const char *relative,
                 unsigned char **out, size_t *length, size_t max_bytes,
                 char *error, size_t cap) {
    *out=NULL;*length=0;
    uint64_t lock_started = monotonic_ms();
    if (lock_read_session(error, cap) < 0) return -1;
    trace_stage("read_session_lock", lock_started, host, export_path, relative, "outcome=acquired");
    struct nfs_context *nfs=get_read_session(host,export_path,error,cap);
    if (!nfs) {pthread_mutex_unlock(&read_session_lock);return -1;}
    if (cancelled_error(error,cap)) { pthread_mutex_unlock(&read_session_lock); return -1; }
    struct nfsfh *fh=NULL;
    uint64_t open_started = monotonic_ms();
    if(nfs_open(nfs,relative,O_RDONLY,&fh)!=0) {
        trace_stage("file_open", open_started, host, export_path, relative, "outcome=error");
        err(error,cap,"nfs_open",nfs);invalidate_read_session();
        pthread_mutex_unlock(&read_session_lock);return -1;
    }
    trace_stage("file_open", open_started, host, export_path, relative, "outcome=ok");
    size_t capbytes=64*1024,used=0;
    if(max_bytes<capbytes)capbytes=max_bytes;
    unsigned char *bytes=malloc(capbytes?capbytes:1);
    if(!bytes) {snprintf(error,cap,"NFS out of memory");close_read_handle(nfs,fh);
        pthread_mutex_unlock(&read_session_lock);return -1;}
    int failed=0;
    uint64_t read_started = monotonic_ms();
    int read_calls = 0;
    for (;;) {
        if (cancelled_error(error, cap)) { failed=1; break; }
        if(used==capbytes) {
            if(capbytes>=max_bytes) {snprintf(error,cap,"NFS image exceeds 128 MiB safety limit");failed=1;break;}
            size_t next=capbytes*2;if(next>max_bytes)next=max_bytes;
            unsigned char *more=realloc(bytes,next);
            if(!more){snprintf(error,cap,"NFS out of memory");failed=1;break;}
            bytes=more;capbytes=next;
        }
        uint64_t call_started = monotonic_ms();
        size_t chunk=capbytes-used;
        if (scan_cancel && chunk>64*1024) chunk=64*1024;
        int got;
#if defined(PIC_LIBNFS_LEGACY_READ_ORDER)
        got=nfs_read(nfs,fh,(uint64_t)chunk,bytes+used);
#else
        got=nfs_read(nfs,fh,bytes+used,chunk);
#endif
        read_calls++;
        if (trace_enabled()) {
            char detail[128];
            snprintf(detail, sizeof detail,
                     "iteration=%d outcome=%s bytes=%d",
                     read_calls, got < 0 ? "error" : "ok", got);
            trace_stage("rpc_read", call_started, host, export_path, relative, detail);
        }
        if (read_calls == 1) {
            char detail[64];
            snprintf(detail, sizeof detail, "outcome=%s bytes=%d", got < 0 ? "error" : "ok", got);
            trace_stage("first_read", call_started, host, export_path, relative, detail);
        }
        if(got<0){err(error,cap,"nfs_read",nfs);failed=1;break;}
        if(got==0)break;
        used+=(size_t)got;
    }
    int canceled=scan_cancelled();
    if (canceled) { snprintf(error,cap,"NFS scan cancelled"); failed=1; }
    int close_status=close_read_handle(nfs,fh);
    if (scan_cancelled()) { canceled=1; failed=1; snprintf(error,cap,"NFS scan cancelled"); }
    char read_detail[96];
    snprintf(read_detail, sizeof read_detail, "outcome=%s bytes=%zu calls=%d",
             failed ? "error" : "ok", used, read_calls);
    trace_stage("file_read_complete", read_started, host, export_path, relative, read_detail);
    if (close_status<0 && !failed) {err(error,cap,"nfs_close",nfs);failed=1;}
    if(failed){
        /* Resource/memory/size errors do not imply a broken NFS session. */
        if (canceled || close_status<0 || strstr(error,"nfs_read:") || strstr(error,"nfs_close:"))
            invalidate_read_session();
        free(bytes);pthread_mutex_unlock(&read_session_lock);return -1;
    }
    *out=bytes;*length=used;pthread_mutex_unlock(&read_session_lock);return 0;
}

int pic_nfs_read_range(const char *host, const char *export_path, const char *relative,
                       uint64_t offset, size_t requested, unsigned char **out, size_t *length,
                       char *error, size_t cap) {
    *out=NULL; *length=0;
    if (lock_read_session(error, cap) < 0) return -1;
    struct nfs_context *nfs=get_read_session(host,export_path,error,cap);
    if (!nfs) { pthread_mutex_unlock(&read_session_lock); return -1; }
    if (cancelled_error(error,cap)) { pthread_mutex_unlock(&read_session_lock); return -1; }
    struct nfsfh *fh=NULL;
    if (nfs_open(nfs,relative,O_RDONLY,&fh)!=0) { err(error,cap,"nfs_open",nfs); invalidate_read_session(); pthread_mutex_unlock(&read_session_lock); return -1; }
    if (cancelled_error(error,cap)) { close_read_handle(nfs,fh); invalidate_read_session(); pthread_mutex_unlock(&read_session_lock); return -1; }
    uint64_t position=0;
    if (nfs_lseek(nfs,fh,(int64_t)offset,SEEK_SET,&position)!=0) { err(error,cap,"nfs_lseek",nfs); close_read_handle(nfs,fh); invalidate_read_session(); pthread_mutex_unlock(&read_session_lock); return -1; }
    unsigned char *bytes=malloc(requested ? requested : 1);
    if (!bytes) { snprintf(error,cap,"NFS out of memory"); close_read_handle(nfs,fh); pthread_mutex_unlock(&read_session_lock); return -1; }
    size_t used=0;
    while (used < requested) {
        if (cancelled_error(error, cap)) {
            free(bytes); close_read_handle(nfs,fh); invalidate_read_session(); pthread_mutex_unlock(&read_session_lock); return -1;
        }
        size_t chunk=requested-used;
        if (scan_cancel && chunk>64*1024) chunk=64*1024;
        int got;
#if defined(PIC_LIBNFS_LEGACY_READ_ORDER)
        got=nfs_read(nfs,fh,(uint64_t)chunk,bytes+used);
#else
        got=nfs_read(nfs,fh,bytes+used,chunk);
#endif
        if (got < 0) { err(error,cap,"nfs_read",nfs); free(bytes); close_read_handle(nfs,fh); invalidate_read_session(); pthread_mutex_unlock(&read_session_lock); return -1; }
        if (got == 0) break;
        used+=(size_t)got;
    }
    if (cancelled_error(error,cap)) {
        free(bytes); close_read_handle(nfs,fh); invalidate_read_session(); pthread_mutex_unlock(&read_session_lock); return -1;
    }
    close_read_handle(nfs,fh);
    if (cancelled_error(error,cap)) { free(bytes); invalidate_read_session(); pthread_mutex_unlock(&read_session_lock); return -1; }
    pthread_mutex_unlock(&read_session_lock);
    *out=bytes; *length=used;
    if (trace_enabled()) fprintf(stderr,"PIC_NFS_RANGE offset=%llu requested=%zu received=%zu host=%s relative=%s\n",(unsigned long long)offset,requested,used,host,relative);
    return 0;
}

/* Scanner calls stat repeatedly on one worker thread. Reuse the libnfs
 * userspace session per worker, not one TCP session per photo. Never shared
 * across threads. An NFS error invalidates this cached context. */
static _Thread_local struct nfs_context *stat_session=NULL;
static _Thread_local char stat_host[256]={0};
static _Thread_local char stat_export[4096]={0};
int pic_nfs_stat(const char *host, const char *export_path, const char *relative,
                 uint64_t *size, int64_t *mtime, int *is_dir,
                 char *error, size_t cap) {
    if (cancelled_error(error,cap)) return -1;
    if (!stat_session || strcmp(stat_host,host) || strcmp(stat_export,export_path)) {
        if (stat_session) {nfs_destroy_context(stat_session);stat_session=NULL;}
        stat_session=open_session(host,export_path,error,cap);
        if (!stat_session) return -1;
        snprintf(stat_host,sizeof stat_host,"%s",host);
        snprintf(stat_export,sizeof stat_export,"%s",export_path);
    }
    struct nfs_stat_64 st={0};
    int result=nfs_stat64(stat_session,relative,&st);
    if (result) {
        err(error,cap,"nfs_stat64",stat_session);
        nfs_destroy_context(stat_session);
        stat_session=NULL;
        stat_host[0]=0;
        stat_export[0]=0;
        return -1;
    }
    *size=st.nfs_size;
    *mtime=(int64_t)st.nfs_mtime;
    *is_dir=(st.nfs_mode & S_IFMT)==S_IFDIR;
    return 0;
}
