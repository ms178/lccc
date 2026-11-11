/* 16-byte objects (`__int128` / `_Float128`) through `__seg_gs` access.
 *
 * Two defects are pinned here, both of which corrupted memory silently:
 *
 *  1. `__int128 __seg_gs x` lost its address space.  The wide base token
 *     returned straight out of the parser's base-token loop, skipping the
 *     trailing qualifier/storage-class loop, so the compiled access was an
 *     ordinary unsegmented one.  A `__int128 const` lost its `const` for the
 *     same reason.
 *  2. Once the segment survived, the segment emitters still moved a single
 *     8-byte half: a store left the high word untouched, a load read whatever
 *     the destination slot already held.
 *
 * Address model (shared with tests/bench/seg_gs.h): every object the segment
 * accesses touch is published twice -- once at its own address and once in a
 * mirror one 4 GiB window above it -- and the GS base is pointed at that
 * window.  `%gs:A` therefore lands on A's mirror, while a plain access lands
 * on A itself.  A dropped segment prefix is thus a *different value*, not a
 * wild read, so each check below can distinguish the two.
 */
#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <stdlib.h>
#include <sys/mman.h>
#include <sys/syscall.h>
#include <unistd.h>

#define ARCH_SET_GS 0x1001
#define DISP ((uintptr_t)1 << 32)

static void map_mirror(const void *g, size_t n) {
    uintptr_t lo = (uintptr_t)g + DISP, end = lo + n;
    for (uintptr_t p = lo & ~(uintptr_t)4095; p < end; p += 4096) {
        void *w = (void *)p;
        void *m = mmap(w, 4096, PROT_READ | PROT_WRITE,
                       MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED_NOREPLACE, -1, 0);
        if (m == MAP_FAILED) {
            /* EEXIST: an earlier object already published this page. */
            if (errno == EEXIST) continue;
            perror("mmap");
            exit(2);
        }
        if (m != w) {
            munmap(m, 4096);
            if (mmap(w, 4096, PROT_READ | PROT_WRITE,
                     MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED, -1, 0) != w)
                exit(2);
        }
    }
}

/* Storage whose segment-relative addresses the pointers below hold. */
static __int128 slot_a[2], slot_b[2];
static _Float128 fslot[2];
static __int128 arr[4];
static __int128 __seg_gs gwide;
static const __int128 konst = 7;

static int failures;
#define CHECK(cond, name) do { \
    if (cond) printf("ok   %s\n", name); \
    else { printf("FAIL %s\n", name); failures++; } \
} while (0)

static __int128 mix(__int128 v) { return (v << 3) ^ (v + 0x5a5a5a5a5a5a5a5aLL); }

/* A copy through pointers in the segment: the shape that first exposed the
 * 8-byte-only transfer. */
__attribute__((noinline)) static void copy_through(
    __int128 __seg_gs *dst, const __int128 __seg_gs *src) {
    *dst = *src;
}

/* Store a value that arrives in registers and never has a slot of its own. */
__attribute__((noinline)) static void copy_via_value(__int128 __seg_gs *dst,
                                                     __int128 v) {
    *dst = v;
}

__attribute__((noinline)) static __int128 read_back(const __int128 __seg_gs *p) {
    return *p;
}

int main(void) {
    map_mirror(slot_a, sizeof slot_a);
    map_mirror(slot_b, sizeof slot_b);
    map_mirror(fslot, sizeof fslot);
    map_mirror(arr, sizeof arr);
    /* Its mirror is published through an explicit integer cast: the object's
     * own address is segment-relative, which is the whole point of the test. */
    map_mirror((const void *)(uintptr_t)&gwide, sizeof gwide);

    if (syscall(SYS_arch_prctl, ARCH_SET_GS, (void *)DISP) != 0) {
        perror("arch_prctl");
        return 2;
    }

    /* Segment-relative addresses: the objects themselves, reached through the
     * GS window by the base above. */
    __int128 __seg_gs *a = (__int128 __seg_gs *)slot_a;
    __int128 __seg_gs *b = (__int128 __seg_gs *)slot_b;
    __int128 want = ((__int128)0x1122334455667788ULL << 64) | 0x99aabbccddeeff00ULL;

    /* 1. Plain store/load through a segment pointer. */
    *a = want;
    CHECK(*a == want, "store/load through __seg_gs pointer keeps all 16 bytes");

    /* 2. Pointer-to-pointer copy: both halves must travel. */
    copy_through(b, a);
    CHECK(read_back(b) == want, "segment 16-byte copy through two pointers");

    /* 3. Store from a register-born value. */
    *b = 0;
    copy_via_value(b, mix(want));
    CHECK(read_back(b) == mix(want), "16-byte store from a register-born value");

    /* 4. A wide constant image, including a non-zero high half. */
    *b = ((__int128)0x0123456789abcdefLL << 64) | 0xfedcba9876543210ULL;
    CHECK(read_back(b) == (((__int128)0x0123456789abcdefLL << 64) |
                           0xfedcba9876543210ULL),
          "16-byte constant store");
    *b = 0;
    CHECK(read_back(b) == 0, "16-byte zero store");

    /* 5. A `_Float128` object in the segment keeps all 16 bytes.  The value
     * stored is a bit pattern the compiler cannot round to a smaller
     * precision, so a store that transfers only the low eight bytes leaves a
     * visibly different low half -- and the comparison is back through the
     * generic view, because casting binary128 to __int128 is a VALUE
     * conversion rather than the bit copy under test. */
    static const unsigned char fpat[16] = {0x00, 0x01, 0x02, 0x03, 0x04, 0x05,
                                           0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b,
                                           0x0c, 0x0d, 0x0e, 0x0f};
    _Float128 fval;
    memcpy(&fval, fpat, 16);
    _Float128 __seg_gs *f = (_Float128 __seg_gs *)fslot;
    memset(fslot, 0xff, sizeof fslot);
    *f = fval;
    /* Read back THROUGH the segment: the generic `fslot` is the other view and
     * must still hold the 0xff filler. */
    _Float128 fback = *f;
    CHECK(memcmp(&fback, fpat, 16) == 0,
          "_Float128 __seg_gs store keeps all 16 bytes");

    /* 6. Indexed segment accesses at 16-byte stride. */
    __int128 __seg_gs *base = (__int128 __seg_gs *)arr;
    for (int i = 0; i < 4; i++) base[i] = want + i;
    int idx_ok = 1;
    for (int i = 0; i < 4; i++)
        if (base[i] != want + i) idx_ok = 0;
    CHECK(idx_ok, "indexed 16-byte segment accesses");

    /* 7. A whole `__seg_gs` object, not just a pointee: this is the shape whose
     * address space the parser used to drop. */
    gwide = want;
    CHECK(gwide == want, "__int128 __seg_gs object keeps its address space");
    gwide = gwide + konst;
    CHECK(gwide == want + 7, "const __int128 operand folds with the segment");

    /* 8. The generic view must NOT see any of it: %gs really was used. */
    CHECK(slot_a[0] != want && slot_b[0] != mix(want),
          "accesses went through %gs, not the generic address");

    if (failures == 0) {
        printf("PASSED (0 failures)\n");
        return 0;
    }
    printf("FAILED (%d failures)\n", failures);
    return 1;
}
