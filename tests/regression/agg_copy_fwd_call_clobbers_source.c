/* Aggregate snapshot copies must not be forwarded across opaque writes.
 *
 * SQLite 3.53.4 memjrnlCreateFile (inlined into memjrnlWrite):
 *
 *   MemJournal copy = *p;
 *   memset(p, 0, sizeof(MemJournal));
 *   rc = sqlite3OsOpen(copy.pVfs, copy.zJournal, pReal, copy.flags, 0);
 *
 * Both aggregate_copy_forward and aggregate_sroa (transform 1) forwarded the
 * `copy.*` reads back to `p`'s memory: their source-mutation window scans
 * only recognized Store/Memcpy instructions as writes, and libc memset is
 * lowered as a plain Call. The copy was then elided as dead, the reads
 * observed the zeroed struct, and speedtest1 --testset json crashed calling
 * a NULL xOpen function pointer.
 *
 * Fixed by treating calls, inline asm, atomics, va-machinery, and impure
 * intrinsics as opaque memory writes: a snapshot window spanning one may
 * only survive when the source is a provably non-escaping alloca.
 *
 * The struct layout mirrors MemJournal closely enough to keep the
 * function-pointer field at a nonzero offset.
 */
#include <stdio.h>
#include <string.h>

typedef struct VFS {
    int ver;
    int (*xOpen)(struct VFS *, const char *, void *, int, int *);
} VFS;

typedef struct MJ {
    void *pMethod;
    long e1, e2, e3, e4;
    int nChunkSize;
    int nSpill;
    long pFirst;
    int flags;
    int pad;
    VFS *pVfs;
    const char *zJournal;
} MJ;

static int opened;

static int myOpen(VFS *v, const char *z, void *f, int fl, int *out) {
    (void)v;
    (void)f;
    (void)out;
    opened = 1;
    printf("open z=%s flags=%d\n", z, fl);
    return 0;
}

static int create_file(MJ *p) {
    MJ copy = *p;
    memset(p, 0, sizeof(MJ));
    return copy.pVfs->xOpen(copy.pVfs, copy.zJournal, p, copy.flags & 0x1087f7f,
                            0);
}

int main(void) {
    static VFS v = { 3, myOpen };
    MJ m;
    memset(&m, 0, sizeof m);
    m.pVfs = &v;
    m.zJournal = "jrnl";
    m.flags = 0x806;
    int rc = create_file(&m);
    printf("rc=%d opened=%d pVfs=%p\n", rc, opened, (void *)m.pVfs);
    return rc != 0 || !opened;
}
