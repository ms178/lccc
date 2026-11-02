/* Runtime, self-checking contract for pointer arithmetic on decayed __seg_gs
 * arrays and member arrays. Same mirror technique as seg_gs_audit.c: the GS
 * base is set to GS_DISP, the bytes at (&object + GS_DISP) are mapped, and a
 * correct %gs access writes the mirror. Each check reads the mirror with an
 * ordinary pointer, so a generic (missed-segment) access fails the check.
 *
 * Covered forms: array + n, n + array, array - n, pointer += n, pointer to a
 * named-space array row (row[0] + n), a conditional of two array offsets,
 * a member-array decay with and without an offset, and pointer difference.
 */
#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/mman.h>
#include <sys/syscall.h>
#include <unistd.h>

#define ARCH_SET_GS 0x1001
#define GS_DISP ((uintptr_t)1 << 32)
#define MIR(g) ((void *)((char *)(g) + GS_DISP))

struct box { long inner[4]; long pad; };

long __seg_gs dec_arr[8];
struct box __seg_gs boxes[2];
long __seg_gs grid[3][4];

extern long dec_g[] __asm__("dec_arr");
extern struct box bx_g[] __asm__("boxes");
extern long grid_g[3][4] __asm__("grid");

static int failures;

#define CHECK(cond, name)                                  \
    do {                                                   \
        if (cond) printf("ok   %s\n", name);               \
        else { printf("FAIL %s\n", name); failures++; }    \
    } while (0)

static void map_mirror(void *g, size_t n) {
    uintptr_t lo = (uintptr_t)g + GS_DISP;
    uintptr_t page = lo & ~(uintptr_t)4095;
    size_t len = ((lo + n - page) + 4095) & ~(size_t)4095;
    void *m = mmap((void *)page, len, PROT_READ | PROT_WRITE,
                   MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED_NOREPLACE, -1, 0);
    if (m != (void *)page && errno != EEXIST) {
        perror("mmap");
        _Exit(2);
    }
}

__attribute__((noinline)) static long f_init_member(void) {
    long __seg_gs *p = boxes[0].inner + 1;      /* member decay + n */
    *p = 11;
    return boxes[0].inner[1];
}
__attribute__((noinline)) static long f_init_member_plain(void) {
    long __seg_gs *p = boxes[1].inner;          /* member decay */
    p[2] = 12;
    return boxes[1].inner[2];
}
__attribute__((noinline)) static long f_assign_plus(void) {
    long __seg_gs *p;
    p = dec_arr;
    p += 7;                                     /* compound pointer add */
    *p = 21;
    return dec_arr[7];
}
__attribute__((noinline)) static long f_loop_var(long n) {
    long s = 0;
    for (long i = 0; i < n; i++) {
        long __seg_gs *q = dec_arr + i;         /* variable offset */
        *q = i * 3;
    }
    for (long i = 0; i < n; i++) s += *(dec_arr + i);
    return s;
}
__attribute__((noinline)) static long f_sub(void) {
    long __seg_gs *end = dec_arr + 8;
    long __seg_gs *beg = end - 8;               /* array - n */
    return (long)(end - beg);
}
__attribute__((noinline)) static long f_rowptr(void) {
    long __seg_gs (*row)[4] = grid + 1;         /* pointer to named row */
    long __seg_gs *cell = row[0] + 2;           /* row decay + n */
    *cell = 33;
    return grid[1][2] + (long)(row - grid);
}
__attribute__((noinline)) static long f_cond(int c) {
    long __seg_gs *p = c ? dec_arr + 5 : dec_arr + 6;
    *p = 44;
    return dec_arr[5] + dec_arr[6];
}
/* Dereference of an arithmetic result on a named-space array: the read itself
 * must use %gs, not only the pointer type. Seeds: boxes[1].inner[1] = 211 and
 * dec_arr[1] = 3 (written by f_loop_var). */
__attribute__((noinline)) static long f_deref_forms(void) {
    return *(boxes[1].inner + 1) * 1000 + *((7 + dec_arr) - 6);
}
__attribute__((noinline)) static long f_deref_sum(void) {
    long __seg_gs *p = &dec_arr[0] + 4;
    return *p;
}

int main(void) {
    map_mirror(dec_g, sizeof(long) * 8);
    map_mirror(bx_g, sizeof(struct box) * 2);
    map_mirror(grid_g, sizeof(long) * 12);
    if (syscall(SYS_arch_prctl, ARCH_SET_GS, (void *)GS_DISP) != 0) {
        perror("arch_prctl");
        return 2;
    }

    /* Seed the mirrors through ordinary pointers. */
    volatile long *dm = MIR(dec_g);
    for (int i = 0; i < 8; i++) dm[i] = 100 + i;
    volatile struct box *bm = MIR(bx_g);
    for (int k = 0; k < 2; k++) {
        for (int j = 0; j < 4; j++) bm[k].inner[j] = 200 + 10 * k + j;
        bm[k].pad = 0;
    }
    volatile long *gm = MIR(grid_g);
    for (int r = 0; r < 3; r++)
        for (int c = 0; c < 4; c++) gm[r * 4 + c] = 300 + 10 * r + c;

    CHECK(f_init_member() == 11, "member_decay_offset_value");
    CHECK(bm[0].inner[1] == 11, "member_decay_offset_mirror");
    CHECK(f_init_member_plain() == 12, "member_decay_value");
    CHECK(bm[1].inner[2] == 12, "member_decay_mirror");
    CHECK(f_assign_plus() == 21, "pointer_plus_assign_value");
    CHECK(dm[7] == 21, "pointer_plus_assign_mirror");
    CHECK(f_loop_var(5) == 30, "variable_offset_sum");
    CHECK(dm[4] == 12, "variable_offset_mirror");
    CHECK(f_deref_forms() == 211003, "deref_of_arithmetic_read");
    CHECK(f_sub() == 8, "array_minus_n_difference");
    CHECK(f_rowptr() == 34, "row_pointer_decay");
    CHECK(gm[1 * 4 + 2] == 33, "row_pointer_mirror");
    CHECK(f_cond(1) == 44 + 106, "conditional_offset_value");
    CHECK(dm[5] == 44, "conditional_offset_mirror");
    CHECK(f_deref_sum() == 12, "address_plus_n_read");

    printf("%s (%d failure%s)\n", failures ? "FAILED" : "PASSED", failures, failures == 1 ? "" : "s");
    return failures ? 1 : 0;
}
