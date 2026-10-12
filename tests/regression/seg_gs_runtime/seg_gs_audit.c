/* Runtime, self-checking contract for x86-64 __seg_gs objects reached through
 * forms the offset-pointer test (seg_gs_runtime.c) does not cover.
 *
 * Object-level access needs real storage behind %gs:<object>. The GS base is
 * set to GS_DISP, and the bytes at (&object + GS_DISP) are mapped as a mirror.
 * A correct %gs access reads and writes the mirror; a generic access reads the
 * object's own (different) bytes, so a missed segment fails the check instead
 * of silently passing. Expected values are computed through the mirror with
 * ordinary pointers, and the program does not depend on any other compiler.
 *
 * Covered (each one was a silent miscompile before the audit fixes):
 *   block-scope extern object, reversed subscript i[p], conditional/comma/
 *   assignment aggregates over a segment, &object typed __seg_gs, pointer to
 *   array with a segment element, vector compound assignment, complex parts,
 *   volatile aggregate read, braced pointer initializer.
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

struct c { long v[4]; };
typedef long v2_t __attribute__((vector_size(16)));

/* Segment objects (file scope). Each generic alias below uses an asm label to
 * name the same symbol, so the test can reach the mirror of the object. */
long __seg_gs ga_obj;
struct c __seg_gs ca_obj;
struct c __seg_gs cb_obj;
long __seg_gs arr_obj[4];
v2_t __seg_gs vg_obj;
_Complex double __seg_gs cg_obj;
volatile struct c __seg_gs vs_obj;
/* The qualifier after '*' places the pointer object itself in %gs. */
long *__seg_gs gq_obj;

extern long ga_g[] __asm__("ga_obj");
extern struct c ca_g[] __asm__("ca_obj");
extern struct c cb_g[] __asm__("cb_obj");
extern long arr_g[] __asm__("arr_obj");
extern v2_t vg_g[] __asm__("vg_obj");
extern _Complex double cg_g[] __asm__("cg_obj");
extern struct c vs_g[] __asm__("vs_obj");
extern long *gq_g[] __asm__("gq_obj");

static long garr[2] = { 55, 66 };

/* Generic (ordinary) aggregate used as the other arm of a conditional. */
static struct c gen_c = { { 70, 71, 72, 73 } };

/* Pointers into the segment, stored in generic memory. */
static long __seg_gs *gp;
static long __seg_gs (*pa)[4];
static struct c __seg_gs *pglob = &ca_obj;       /* &object typed __seg_gs */
static struct c __seg_gs *parr[1] = { &cb_obj }; /* braced initializer */
static struct c __seg_gs *pbrace = { &ca_obj };

static int failures;

#define CHECK(cond, name)                                  \
    do {                                                   \
        if (cond) printf("ok   %s\n", name);               \
        else { printf("FAIL %s\n", name); failures++; }    \
    } while (0)

/* Objects that share a page share one mirror page: the only mapping that can
 * already exist at GS_DISP + object is an earlier mirror, so EEXIST is fine. */
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

/* ---- block-scope extern names the file-scope segment object ---- */
__attribute__((noinline)) static long blk_read(void) {
    extern long __seg_gs ga_obj;
    return ga_obj;
}
__attribute__((noinline)) static void blk_write(long v) {
    extern long __seg_gs ga_obj;
    ga_obj = v;
}

/* ---- reversed subscript: i[p] is *(i + p) ---- */
__attribute__((noinline)) static long rev_ptr(long i) { return i[gp]; }
__attribute__((noinline)) static long rev_arr(void) { return 2[arr_obj]; }

/* ---- aggregate conditional / comma / assignment over a segment ---- */
__attribute__((noinline)) static struct c sel_seg_seg(int k) { return k ? ca_obj : cb_obj; }
__attribute__((noinline)) static struct c sel_mixed(int k) { return k ? ca_obj : gen_c; }
__attribute__((noinline)) static long sel_member(int k) { return (k ? ca_obj : cb_obj).v[1]; }
__attribute__((noinline)) static struct c comma_ret(void) { return ((void)0, ca_obj); }
__attribute__((noinline)) static struct c assign_ret(void) {
    struct c x;
    x = (cb_obj = ca_obj);
    return x;
}

/* ---- &segment-object keeps its space; the pointer is usable ---- */
__attribute__((noinline)) static long glob_ptr_read(void) { return pglob->v[2]; }

/* ---- pointer to array of segment elements ---- */
__attribute__((noinline)) static long pa_read(int i) { return (*pa)[i]; }
__attribute__((noinline)) static void pa_write(int i, long v) { (*pa)[i] = v; }

/* ---- vector compound assignment and whole-vector assignment ---- */
__attribute__((noinline)) static void vec_add(v2_t x) { vg_obj += x; }
__attribute__((noinline)) static void vec_mul(v2_t x) { vg_obj = vg_obj * x; }

/* ---- complex parts of a segment object ---- */
__attribute__((noinline)) static double cx_re(void) { return __real__ cg_obj; }
__attribute__((noinline)) static double cx_im(void) { return __imag__ cg_obj; }
__attribute__((noinline)) static void cx_set(void) { cg_obj = 1.5 + 2.5i; }
__attribute__((noinline)) static void cx_setim(double d) { __imag__ cg_obj = d; }

/* ---- volatile aggregate read from a segment ---- */
__attribute__((noinline)) static struct c vol_read(void) {
    struct c t;
    t = vs_obj;
    return t;
}

/* ---- pointer object in %gs, pointee generic ---- */
__attribute__((noinline)) static long d1_read(void) { return *gq_obj; }
__attribute__((noinline)) static void d1_write(long v) { *gq_obj = v; }

/* ---- braced pointer initializers ---- */
__attribute__((noinline)) static long brace_read(void) { return parr[0]->v[3] + pbrace->v[0]; }

/* Pointer arithmetic on a decayed named-space array stays in %gs, both as
 * array + n and as n + array, and pointer difference between them is exact. */
__attribute__((noinline)) static long arr_arith_read(void) {
    long __seg_gs *p = arr_obj + 2;
    long __seg_gs *q = 1 + arr_obj;
    return *p * 1000 + *q + (long)((arr_obj + 3) - arr_obj);
}
__attribute__((noinline)) static void arr_arith_write(void) {
    long __seg_gs *w = 3 + arr_obj;
    *w = 555;
}

int main(void) {
    setvbuf(stdout, NULL, _IONBF, 0);
    map_mirror(ga_g, sizeof(long));
    map_mirror(ca_g, sizeof(struct c));
    map_mirror(cb_g, sizeof(struct c));
    map_mirror(arr_g, sizeof(long) * 4);
    map_mirror(vg_g, sizeof(v2_t));
    map_mirror(cg_g, sizeof(_Complex double));
    map_mirror(vs_g, sizeof(struct c));
    map_mirror(gq_g, sizeof(long *));
    if (syscall(SYS_arch_prctl, ARCH_SET_GS, (void *)GS_DISP) != 0) {
        perror("arch_prctl");
        return 2;
    }

    /* Seed the mirror through ordinary pointers. */
    *(volatile long *)MIR(ga_g) = 42;
    volatile struct c *ca_m = MIR(ca_g), *cb_m = MIR(cb_g);
    *ca_m = (struct c){ { 10, 11, 12, 13 } };
    *cb_m = (struct c){ { 20, 21, 22, 23 } };
    volatile long *arr_m = MIR(arr_g);
    for (int i = 0; i < 4; i++) arr_m[i] = 100 + i;
    *(volatile v2_t *)MIR(vg_g) = (v2_t){ 3, 4 };
    *(volatile struct c *)MIR(vs_g) = (struct c){ { 90, 91, 92, 93 } };

    /* Generic-to-segment conversion, deliberately through an integer: a
     * direct pointer cast draws the disjoint-address-space warning, which is
     * the intended diagnostic for that cast and is covered by
     * check_seg_cast_warning.sh, not by this runtime fixture. */
    gp = (long __seg_gs *)(uintptr_t)arr_g;
    pa = (long __seg_gs (*)[4])(uintptr_t)arr_g;

    /* Block-scope extern */
    CHECK(blk_read() == 42, "blk_extern_read");
    blk_write(77);
    CHECK(*(volatile long *)MIR(ga_g) == 77, "blk_extern_write");

    /* Reversed subscript */
    CHECK(rev_ptr(2) == 102 && rev_ptr(0) == 100, "rev_subscript_ptr");
    CHECK(rev_arr() == 102, "rev_subscript_array");

    /* Conditional, comma and assignment aggregates */
    struct c r1 = sel_seg_seg(1), r0 = sel_seg_seg(0);
    CHECK(r1.v[0] == 10 && r0.v[0] == 20, "cond_seg_both_arms");
    struct c m1 = sel_mixed(1), m0 = sel_mixed(0);
    CHECK(m1.v[1] == 11 && m0.v[1] == 71, "cond_mixed_arms");
    CHECK(sel_member(1) == 11 && sel_member(0) == 21, "cond_member_of_result");
    struct c cm = comma_ret();
    CHECK(cm.v[3] == 13, "comma_aggregate_seg");
    struct c as = assign_ret();
    CHECK(as.v[2] == 12 && cb_m->v[2] == 12, "assign_aggregate_seg");

    /* &segment object */
    CHECK(glob_ptr_read() == 12, "addrof_seg_object");

    /* Pointer to array of segment elements */
    CHECK(pa_read(3) == 103 && pa_read(0) == 100, "ptr_to_array_read");
    pa_write(1, 555);
    CHECK(arr_m[1] == 555, "ptr_to_array_write");

    /* Vector compound and whole assignment */
    vec_add((v2_t){ 1, 1 });
    v2_t vv = *(volatile v2_t *)MIR(vg_g);
    CHECK(vv[0] == 4 && vv[1] == 5, "vector_compound_add");
    vec_mul((v2_t){ 2, 3 });
    vv = *(volatile v2_t *)MIR(vg_g);
    CHECK(vv[0] == 8 && vv[1] == 15, "vector_whole_mul");

    /* Complex parts */
    volatile _Complex double *cg_m = MIR(cg_g);
    CHECK(*cg_m == 0.0, "complex_zero_init");
    cx_set();
    CHECK(__real__ *cg_m == 1.5 && __imag__ *cg_m == 2.5, "complex_store");
    CHECK(cx_re() == 1.5 && cx_im() == 2.5, "complex_load");
    cx_setim(9.25);
    CHECK(__imag__ *cg_m == 9.25 && __real__ *cg_m == 1.5, "complex_imag_store");

    /* Volatile aggregate read */
    struct c vr = vol_read();
    CHECK(vr.v[0] == 90 && vr.v[3] == 93, "volatile_aggregate_read");

    /* the pointer object lives in the segment; its pointee is generic */
    *(long **)MIR(gq_g) = garr;
    CHECK(d1_read() == 55, "ptr_object_seg_read");
    d1_write(77);
    CHECK(garr[0] == 77, "ptr_object_seg_deref_write");

    /* Braced pointer initializers */
    CHECK(brace_read() == 13 + 10, "braced_pointer_init");

    /* Decayed-array pointer arithmetic. arr[2] is 102 and arr[1] was set to 555
     * by ptr_to_array_write above; the difference is 3. */
    CHECK(arr_arith_read() == 102000 + 555 + 3, "decay_arith_read");
    arr_arith_write();
    CHECK(arr_m[3] == 555, "decay_arith_write");

    printf("%s (%d failure%s)\n", failures ? "FAILED" : "PASSED", failures, failures == 1 ? "" : "s");
    return failures ? 1 : 0;
}
