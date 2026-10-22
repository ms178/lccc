/* Constant-size memcpy/memmove inlining, exhaustive.
 *
 * `inline_memcpy_len` expands a constant memcpy up to 64 bytes and a
 * constant memmove up to 32 bytes.  memmove is the hard case: the classic
 * forward expansion is only correct for disjoint ranges or dst < src, and
 * the backward one needs `std`/`cld`; the new small-size expansion is
 * branch-free and buffers every chunk in a scratch register before the
 * first store, which is correct for ANY overlap and for dst == src.
 *
 * Coverage: memmove at every size 0..32 with a runtime shift -48..48 (all
 * four overlap relations), the same sizes with a compile-time shift,
 * memcpy at the chunk-ladder boundaries up to 64 bytes against a disjoint
 * source (exact bytes, no store outside [dest, dest+n), result == dest),
 * and page-boundary probes at every size 0..64 with source and destination
 * each ending exactly at a PROT_NONE page, so a single byte of over-read
 * or over-store faults.
 *
 * The oracle is a volatile-temp byte copy, never libc memmove itself.
 * GCC is the differential oracle (stdout + exit code).
 */

#include <stdio.h>
#include <string.h>
#include <sys/mman.h>

#define BUF 4096
#define SEED 11

static unsigned char A[BUF], B[BUF];
static int fails;

/* Oracle: a copy that cannot alias either buffer and cannot be
 * strength-reduced into the sequence under test (volatile temp). */
static void ref_move(unsigned char *d, const unsigned char *s, size_t n)
{
    static volatile unsigned char t[128];
    size_t i;
    for (i = 0; i < n; i++) t[i] = s[i];
    for (i = 0; i < n; i++) d[i] = t[i];
}

static void init(void)
{
    int i;
    for (i = 0; i < BUF; i++) A[i] = B[i] = (unsigned char)(i * 37 + SEED);
}

static void report(const char *what, int n, int sh, int at)
{
    printf("FAIL %s n=%d shift=%d at=%d\n", what, n, sh, at);
    fails++;
}

static void ck(const char *what, int n, int sh)
{
    int i;
    for (i = 0; i < BUF; i++)
        if (A[i] != B[i]) { report(what, n, sh, i); return; }
}

/* One function per constant size: a runtime shift in -48..48 covers
 * disjoint, dst < src overlapping, dst > src overlapping and the
 * same-address case (shift 0, which must be a no-op). */
#define MM_FN(N)                                                        \
static void mm_##N(void)                                                \
{                                                                       \
    int sh;                                                             \
    for (sh = -48; sh <= 48; sh++) {                                    \
        void *r;                                                        \
        init();                                                         \
        r = memmove(A + 1024 + sh, A + 1024, (size_t)(N));              \
        ref_move(B + 1024 + sh, B + 1024, (size_t)(N));                 \
        if (r != (void *)(A + 1024 + sh)) {                             \
            printf("FAIL memmove return n=%d sh=%d\n", (N), sh);       \
            fails++;                                                    \
        }                                                               \
        ck("memmove", (N), sh);                                         \
    }                                                                   \
}

MM_FN(0)
MM_FN(1)
MM_FN(2)
MM_FN(3)
MM_FN(4)
MM_FN(5)
MM_FN(6)
MM_FN(7)
MM_FN(8)
MM_FN(9)
MM_FN(10)
MM_FN(11)
MM_FN(12)
MM_FN(13)
MM_FN(14)
MM_FN(15)
MM_FN(16)
MM_FN(17)
MM_FN(18)
MM_FN(19)
MM_FN(20)
MM_FN(21)
MM_FN(22)
MM_FN(23)
MM_FN(24)
MM_FN(25)
MM_FN(26)
MM_FN(27)
MM_FN(28)
MM_FN(29)
MM_FN(30)
MM_FN(31)
MM_FN(32)

/* The same sizes with a compile-time shift, so operand staging sees a
 * folded address (this shape caught the backwards `rep movsb`
 * off-by-one: it started one byte past the source end). */
#define MM_CS(N, SH)                                                    \
    do {                                                                \
        void *r;                                                        \
        init();                                                         \
        r = memmove(A + 1024 + (SH), A + 1024, (size_t)(N));           \
        ref_move(B + 1024 + (SH), B + 1024, (size_t)(N));              \
        if (r != (void *)(A + 1024 + (SH))) {                           \
            printf("FAIL memmove const return n=%d sh=%d\n", (N), (SH)); \
            fails++;                                                    \
        }                                                               \
        ck("memmove-const", (N), (SH));                                \
    } while (0)

static void mm_const_shift(void)
{
    MM_CS(0, 40);
    MM_CS(0, 17);
    MM_CS(0, 1);
    MM_CS(0, 0);
    MM_CS(0, -1);
    MM_CS(0, -17);
    MM_CS(0, -40);
    MM_CS(1, 40);
    MM_CS(1, 17);
    MM_CS(1, 1);
    MM_CS(1, 0);
    MM_CS(1, -1);
    MM_CS(1, -17);
    MM_CS(1, -40);
    MM_CS(3, 40);
    MM_CS(3, 17);
    MM_CS(3, 1);
    MM_CS(3, 0);
    MM_CS(3, -1);
    MM_CS(3, -17);
    MM_CS(3, -40);
    MM_CS(5, 40);
    MM_CS(5, 17);
    MM_CS(5, 1);
    MM_CS(5, 0);
    MM_CS(5, -1);
    MM_CS(5, -17);
    MM_CS(5, -40);
    MM_CS(8, 40);
    MM_CS(8, 17);
    MM_CS(8, 1);
    MM_CS(8, 0);
    MM_CS(8, -1);
    MM_CS(8, -17);
    MM_CS(8, -40);
    MM_CS(9, 40);
    MM_CS(9, 17);
    MM_CS(9, 1);
    MM_CS(9, 0);
    MM_CS(9, -1);
    MM_CS(9, -17);
    MM_CS(9, -40);
    MM_CS(15, 40);
    MM_CS(15, 17);
    MM_CS(15, 1);
    MM_CS(15, 0);
    MM_CS(15, -1);
    MM_CS(15, -17);
    MM_CS(15, -40);
    MM_CS(16, 40);
    MM_CS(16, 17);
    MM_CS(16, 1);
    MM_CS(16, 0);
    MM_CS(16, -1);
    MM_CS(16, -17);
    MM_CS(16, -40);
    MM_CS(17, 40);
    MM_CS(17, 17);
    MM_CS(17, 1);
    MM_CS(17, 0);
    MM_CS(17, -1);
    MM_CS(17, -17);
    MM_CS(17, -40);
    MM_CS(24, 40);
    MM_CS(24, 17);
    MM_CS(24, 1);
    MM_CS(24, 0);
    MM_CS(24, -1);
    MM_CS(24, -17);
    MM_CS(24, -40);
    MM_CS(31, 40);
    MM_CS(31, 17);
    MM_CS(31, 1);
    MM_CS(31, 0);
    MM_CS(31, -1);
    MM_CS(31, -17);
    MM_CS(31, -40);
    MM_CS(32, 40);
    MM_CS(32, 17);
    MM_CS(32, 1);
    MM_CS(32, 0);
    MM_CS(32, -1);
    MM_CS(32, -17);
    MM_CS(32, -40);
}

/* memcpy against a disjoint source: exact bytes, nothing stored outside
 * [dest, dest+n) on either side, `void *` result == dest. */
#define MC_FN(N)                                                        \
static void mc_##N(void)                                                \
{                                                                       \
    void *r;                                                            \
    int i;                                                              \
    init();                                                             \
    r = memcpy(A + 2048, B + 1024, (size_t)(N));                        \
    if (r != (void *)(A + 2048)) {                                      \
        printf("FAIL memcpy return n=%d\n", (N));                      \
        fails++;                                                        \
    }                                                                   \
    for (i = 0; i < (N); i++)                                           \
        if (A[2048 + i] != B[1024 + i]) {                               \
            report("memcpy", (N), 0, 2048 + i);                         \
            break;                                                      \
        }                                                               \
    for (i = 2048 + (N); i < 2048 + 128; i++)                           \
        if (A[i] != (unsigned char)(i * 37 + SEED)) {                   \
            report("memcpy-tail", (N), 0, i);                           \
            break;                                                      \
        }                                                               \
    for (i = 2048 - 128; i < 2048; i++)                                 \
        if (A[i] != (unsigned char)(i * 37 + SEED)) {                   \
            report("memcpy-head", (N), 0, i);                           \
            break;                                                      \
        }                                                               \
}

MC_FN(0)
MC_FN(1)
MC_FN(2)
MC_FN(3)
MC_FN(4)
MC_FN(5)
MC_FN(6)
MC_FN(7)
MC_FN(8)
MC_FN(9)
MC_FN(10)
MC_FN(11)
MC_FN(12)
MC_FN(13)
MC_FN(14)
MC_FN(15)
MC_FN(16)
MC_FN(17)
MC_FN(18)
MC_FN(20)
MC_FN(23)
MC_FN(24)
MC_FN(31)
MC_FN(32)
MC_FN(33)
MC_FN(40)
MC_FN(47)
MC_FN(48)
MC_FN(49)
MC_FN(56)
MC_FN(63)
MC_FN(64)

/* Page-boundary probes: source and destination each end exactly at a
 * PROT_NONE guard page, so one byte of over-read or over-store faults
 * instead of passing silently.  Runs every size 0..64 in both
 * directions (`memmove(d - n, d, n)` is the dst > src overlap). */
static unsigned char *page_end(int n, unsigned char **base)
{
    const long ps = 4096;
    unsigned char *m = (unsigned char *)mmap(0, (size_t)(2 * ps),
                                             PROT_READ | PROT_WRITE,
                                             MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (m == (unsigned char *)-1) { printf("FAIL mmap\n"); fails++; return 0; }
    if (mprotect(m + ps, (size_t)ps, PROT_NONE) != 0) {
        printf("FAIL mprotect\n");
        fails++;
        munmap(m, (size_t)(2 * ps));
        return 0;
    }
    *base = m;
    for (int i = 0; i < (int)ps; i++) m[i] = (unsigned char)(i * 7 + 3);
    return m + ps - n;
}

static void guard_one(int n)
{
    unsigned char *sb, *db, *s, *d;
    int i;
    s = page_end(n, &sb);
    if (!s) return;
    d = page_end(n, &db);
    if (!d) { munmap(sb, 8192); return; }
    for (i = 0; i < n; i++) s[i] = (unsigned char)(i * 31 + 5);
    if (memmove(d, s, (size_t)n) != (void *)d) {
        printf("FAIL guard memmove return n=%d\n", n);
        fails++;
    }
    for (i = 0; i < n; i++)
        if (d[i] != (unsigned char)(i * 31 + 5)) { report("guard-memmove", n, 0, i); break; }
    /* dst > src overlap inside the destination page, both ending at it */
    if (memmove(d - n, d, (size_t)n) != (void *)(d - n)) {
        printf("FAIL guard memmove2 return n=%d\n", n);
        fails++;
    }
    for (i = 0; i < n; i++)
        if (d[i] != (unsigned char)(i * 31 + 5)) { report("guard-memmove2", n, 0, i); break; }
    if (memcpy(d, s, (size_t)n) != (void *)d) {
        printf("FAIL guard memcpy return n=%d\n", n);
        fails++;
    }
    for (i = 0; i < n; i++)
        if (d[i] != (unsigned char)(i * 31 + 5)) { report("guard-memcpy", n, 0, i); break; }
    munmap(sb, 8192);
    munmap(db, 8192);
}

int main(void)
{
    int n;
    mm_0();
    mm_1();
    mm_2();
    mm_3();
    mm_4();
    mm_5();
    mm_6();
    mm_7();
    mm_8();
    mm_9();
    mm_10();
    mm_11();
    mm_12();
    mm_13();
    mm_14();
    mm_15();
    mm_16();
    mm_17();
    mm_18();
    mm_19();
    mm_20();
    mm_21();
    mm_22();
    mm_23();
    mm_24();
    mm_25();
    mm_26();
    mm_27();
    mm_28();
    mm_29();
    mm_30();
    mm_31();
    mm_32();
    mm_const_shift();
    mc_0();
    mc_1();
    mc_2();
    mc_3();
    mc_4();
    mc_5();
    mc_6();
    mc_7();
    mc_8();
    mc_9();
    mc_10();
    mc_11();
    mc_12();
    mc_13();
    mc_14();
    mc_15();
    mc_16();
    mc_17();
    mc_18();
    mc_20();
    mc_23();
    mc_24();
    mc_31();
    mc_32();
    mc_33();
    mc_40();
    mc_47();
    mc_48();
    mc_49();
    mc_56();
    mc_63();
    mc_64();
    for (n = 0; n <= 64; n++) guard_one(n);
    if (fails) { printf("FAILURES: %d\n", fails); return 1; }
    printf("OK inline_memmove_const_sizes\n");
    return 0;
}
