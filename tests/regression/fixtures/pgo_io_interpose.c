/* Deterministic PGO I/O fault/concurrency oracle, not production runtime. */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/file.h>
#include <unistd.h>

static FILE *profile;
static int profile_fd = -1;
static int locked;

static int fault(const char *name) {
    const char *mode = getenv("PGO_IO_FAULT");
    return mode && !strcmp(mode, name);
}
static int role(const char *name) {
    const char *r = getenv("PGO_IO_ROLE");
    return r && !strcmp(r, name);
}
static void marker(const char *name, int wait) {
    const char *dir = getenv("PGO_IO_SYNC");
    char path[4096];
    if (!dir) return;
    int n = snprintf(path, sizeof(path), "%s/%s", dir, name);
    if (n < 0 || (size_t)n >= sizeof(path)) _exit(96);
    if (wait) {
        for (int i = 0; i < 10000; ++i) {
            if (!access(path, F_OK)) return;
            usleep(1000);
        }
        _exit(97);
    }
    int fd = open(path, O_WRONLY | O_CREAT, 0600);
    if (fd < 0 || close(fd)) _exit(98);
}
FILE *fopen(const char *path, const char *mode) {
    FILE *(*real)(const char *, const char *) = dlsym(RTLD_NEXT, "fopen");
    const char *wanted = getenv("LCCC_PROFILE_FILE");
    int is_profile = wanted && !strcmp(path, wanted);
    if (is_profile && fault("fopen")) { errno = EACCES; return NULL; }
    FILE *f = real(path, mode);
    if (is_profile && f) {
        int (*fd_of)(FILE *) = dlsym(RTLD_NEXT, "fileno");
        profile = f;
        profile_fd = fd_of(f);
    }
    return f;
}
int fileno(FILE *f) {
    int (*real)(FILE *) = dlsym(RTLD_NEXT, "fileno");
    if (f == profile && fault("fileno")) { errno = EBADF; return -1; }
    return real(f);
}
int flock(int fd, int op) {
    int (*real)(int, int) = dlsym(RTLD_NEXT, "flock");
    if (fd == profile_fd && op == LOCK_EX) {
        if (fault("flock")) { errno = ENOLCK; return -1; }
        if (fault("eintr")) { errno = EINTR; return -1; }
    }
    int rc = real(fd, op);
    if (!rc && fd == profile_fd) {
        if (op == LOCK_EX) {
            locked = 1;
            if (role("A")) marker("A-held", 0);
        }
        if (op == LOCK_UN) {
            locked = 0;
            /* Old runtime unlocks before its tail is flushed. Force B's
               complete dump between that unlock and A's final flush. */
            if (role("A")) marker("B-done", 1);
        }
    }
    return rc;
}
int ferror(FILE *f) {
    int (*real)(FILE *) = dlsym(RTLD_NEXT, "ferror");
    if (f == profile && fault("ferror")) { errno = EIO; return 1; }
    return real(f);
}
int fclose(FILE *f) {
    int (*real)(FILE *) = dlsym(RTLD_NEXT, "fclose");
    int is_profile = f == profile;
    if (is_profile && !locked && !getenv("PGO_IO_FAULT"))
        marker("unlocked-close", 0);
    int rc = real(f); /* final flush and descriptor close, in that order */
    if (is_profile) {
        profile = NULL;
        profile_fd = -1;
        locked = 0;
        if (role("B")) marker("B-done", 0);
        if (fault("fclose")) { errno = EIO; return EOF; }
    }
    return rc;
}
