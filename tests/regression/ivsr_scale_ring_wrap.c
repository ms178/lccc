/* SCALE-RING-WRAP: the narrow index ring must survive into the address.
 *
 * `buf[i * 2]` with `uint32_t i` computes the INDEX in `unsigned int` (32-bit,
 * wrapping by definition) and only then converts it to `ptrdiff_t` for the
 * address (C17 6.5.6p8, 6.3.1.3p3).  Two independent transformations used to
 * linearise that arithmetic into the pointer ring, where nothing wraps:
 *
 *   (1) the back end's SIB peel, which folded the `add(i, i)` the canonicalizer
 *       makes of `i * 2` into `scale = 2` of a zero-extended index, and folded
 *       `add(iv, k)` / `sub(iv, k)` into a displacement;
 *   (2) IVSR, whose narrow-unsigned no-wrap proof decoded the loop limit with
 *       `IrConst::to_i64` -- a `uint32_t` bound above 2^31 is stored in its own
 *       ring as a NEGATIVE `I32`, so the proof was handed a negative maximum
 *       and certified exactly the wrap it exists to refuse.
 *
 * Every index arithmetic below is unsigned and every access lands inside the
 * mapping, so each program is defined C, both the wrapped and the linear
 * address are reachable, and only one of them is correct.
 *
 * WHY HALF THE CASES DRIVE AN OPAQUE BOUND
 * ----------------------------------------
 * A constant trip count lets the front end fold the loop away, and a folded
 * access is indexed by a CONSTANT.  Measured on the pre-fix compiler with the
 * 39-shape differential in scripts/wrap_ring_sweep.py: with constant indices it
 * agrees with gcc on 30/30 shapes at -O2 and -O3 and diverges only at -O1; with
 * the index routed through a `volatile` so it survives folding it diverges on
 * 21/39 shapes at -O2 AND -O3, and on 42/78 case-shapes at -O1.  So the
 * miscompile that ships in a default -O2 build is observable only through a
 * loop that stays a loop.  `0x80000003u + (k & 0u)` -- the "opaque" spelling
 * this file used to carry -- is NOT opaque: `k & 0u` folds under SCCP and the
 * loop collapses to four straight-line loads, which is why the class looked
 * like an -O1-only problem.
 *
 * Each case gets its own mapping and its own marker bytes, because the shapes
 * wrap onto different offsets and a shared layout would make one case's correct
 * answer another case's wrong one.
 *
 * Needs LP64: on ILP32 the span cannot be expressed by `size_t` and the narrow
 * ring IS the address ring, so the whole class is sound there by construction
 * and the test skips.
 */
#include <stdint.h>
#include <stdio.h>
#include <sys/mman.h>
#include <unistd.h>

#if UINTPTR_MAX < 0xFFFFFFFFFFFFFFFFull
int main(void) {
    puts("ivsr_scale_ring_wrap: SKIP (needs a 64-bit address space)");
    return 0;
}
#else

#define SPAN ((size_t)0x100000000ull + 16u)

/* Opaque loop bounds.  `volatile` at file scope: no fold can retire them, so
 * the loops stay loops and the index stays a narrow loop-carried recurrence. */
static volatile uint32_t lim_hi = 0x80000003u; /* i: 0x7FFFFFFF .. 0x80000002 */
static volatile uint32_t lim_mid = 0x40000003u; /* i: 0x3FFFFFFF .. 0x40000002 */
static volatile uint32_t lim_ne = 2u; /* i: 0xFFFFFFFE, 0xFFFFFFFF, 0, 1 */
static volatile uint32_t lim_dn = 0xFFFFFFFDu; /* i: 1, 0, 0xFFFFFFFF, 0xFFFFFFFE */

/* --- the shapes under test ------------------------------------------------ */

/* scale 2, opaque bound: products {0xFFFFFFFE, 0x100000000, +2, +4} whose
 * wrapped spellings are {0xFFFFFFFE, 0, 2, 4}. */
__attribute__((noinline)) static unsigned long w_mul2_opaque(const unsigned char *buf) {
    unsigned long s = 0;
    uint32_t lim = lim_hi;
    for (uint32_t i = 0x7FFFFFFFu; i < lim; i++)
        s += (unsigned long)buf[i * 2u];
    return s;
}

/* The same walk with a CONSTANT bound: at -O2 the loop folds away and the
 * accesses are indexed by constants.  Kept because it is the shape the IVSR
 * limit decode bit on, and because it must not be "fixed" into the linear ring
 * either. */
__attribute__((noinline)) static unsigned long w_mul2_const(const unsigned char *buf) {
    unsigned long s = 0;
    for (uint32_t i = 0x7FFFFFFFu; i < 0x80000003u; i++)
        s += (unsigned long)buf[i * 2u];
    return s;
}

/* scale 4, opaque bound: i in {0x3FFFFFFF..0x40000002}, products
 * {0xFFFFFFFC, 0x100000000, +4, +8}, wrapped {0xFFFFFFFC, 0, 4, 8}. */
__attribute__((noinline)) static unsigned long w_mul4_opaque(const unsigned char *buf) {
    unsigned long s = 0;
    uint32_t lim = lim_mid;
    for (uint32_t i = 0x3FFFFFFFu; i < lim; i++)
        s += (unsigned long)buf[i * 4u];
    return s;
}

/* `add(iv, 1)` in the narrow ring -- the displacement peel, and the shape whose
 * refusal costs the most performance, so it is the one most likely to be
 * "optimised" back into a bug.  i in {0xFFFFFFFE, 0xFFFFFFFF, 0, 1} gives
 * i+1 in {0xFFFFFFFF, 0, 1, 2} wrapped, {0xFFFFFFFF, 0x100000000, +1, +2}
 * linear.  The bound is `!=` because the sequence crosses the top of the ring. */
__attribute__((noinline)) static unsigned long w_addconst_opaque(const unsigned char *buf) {
    unsigned long s = 0;
    uint32_t stop = lim_ne;
    for (uint32_t i = 0xFFFFFFFEu; i != stop; i++)
        s += (unsigned long)buf[i + 1u];
    return s;
}

/* `sub(iv, 1)`: the wrap goes DOWN through zero, so a peeled `-1` displacement
 * addresses buf-1 at the third iteration -- below the mapping.  A wrong
 * compiler faults here instead of summing wrongly, which is a louder verdict
 * and the reason this case runs last. */
__attribute__((noinline)) static unsigned long w_subconst_opaque(const unsigned char *buf) {
    unsigned long s = 0;
    uint32_t stop = lim_dn;
    for (uint32_t i = 1u; i != stop; i--)
        s += (unsigned long)buf[i - 1u];
    return s;
}

/* --- CONTROLS: shapes whose correct answer is the LINEAR address ------------ */

/* `sp[i]` converts the index to `ptrdiff_t` BEFORE scaling, so the element
 * address never wraps and the linear marker is correct.  A fix that starts
 * wrapping this shape is a regression, and without a control it is invisible. */
struct pair {
    unsigned char a, b;
};

__attribute__((noinline)) static unsigned long c_struct_element(const unsigned char *buf) {
    unsigned long s = 0;
    const struct pair *sp = (const struct pair *)(const void *)buf;
    uint32_t lim = lim_hi;
    for (uint32_t i = 0x7FFFFFFFu; i < lim; i++)
        s += (unsigned long)sp[i].a;
    return s;
}

/* A 64-bit index expression is evaluated in the pointer ring already, so
 * `(uint64_t)i * 2` does not wrap and the linear marker is correct. */
__attribute__((noinline)) static unsigned long c_u64_index(const unsigned char *buf) {
    unsigned long s = 0;
    uint32_t lim = lim_hi;
    for (uint32_t i = 0x7FFFFFFFu; i < lim; i++)
        s += (unsigned long)buf[(uint64_t)i * 2u];
    return s;
}

/* A masked index cannot reach the top of its ring, so both spellings agree.
 * This is the shape a future range-gated peel must keep folding: refusing it
 * would cost real code for no soundness.  The mask is one bit so the four
 * iterations land on two marked slots and the expectation stays derivable. */
__attribute__((noinline)) static unsigned long c_masked_index(const unsigned char *buf) {
    unsigned long s = 0;
    uint32_t lim = lim_hi;
    for (uint32_t i = 0x7FFFFFFFu; i < lim; i++)
        s += (unsigned long)buf[(i & 1u) * 4u];
    return s;
}

/* --- harness --------------------------------------------------------------- */

struct mark {
    unsigned long long off;
    unsigned char val;
};

struct case_def {
    const char *name;
    unsigned long (*fn)(const unsigned char *);
    const struct mark *marks;
    unsigned nmarks;
    unsigned long want; /* the correct total, per C */
    unsigned long other; /* what the wrong ring would give (diagnostic only) */
};

static int run_case(const struct case_def *c) {
    size_t page = (size_t)sysconf(_SC_PAGESIZE);
    size_t len = SPAN + 2 * page;
    char *map = mmap(0, len, PROT_NONE, MAP_PRIVATE | MAP_ANONYMOUS | MAP_NORESERVE, -1, 0);
    if (map == MAP_FAILED) {
        perror("ivsr_scale_ring_wrap: mmap");
        return -1;
    }
    unsigned char *buf = (unsigned char *)(map + page);
    /* Committed pages at offset 0 (wrapped landing sites), at 0xFFFFF000 (the
     * top of the 32-bit ring, where the descending walk lands) and at 2^32 (the
     * LINEAR landing sites).  Everything between stays PROT_NONE, so a wrong
     * address faults instead of reading zeros. */
    if (mprotect(buf, page, PROT_READ | PROT_WRITE) ||
        mprotect(buf + 0xFFFFF000u, page, PROT_READ | PROT_WRITE) ||
        mprotect(buf + 0x100000000u, page, PROT_READ | PROT_WRITE)) {
        perror("ivsr_scale_ring_wrap: mprotect");
        munmap(map, len);
        return -1;
    }
    for (unsigned i = 0; i < c->nmarks; i++)
        buf[c->marks[i].off] = c->marks[i].val;

    unsigned long got = c->fn(buf);
    munmap(map, len);
    if (got != c->want) {
        if (c->other == c->want)
            /* No alternative total exists: the wrong ring faults instead of
             * summing (see the sub(iv,1) case). */
            printf("ivsr_scale_ring_wrap: FAIL %s: got %lu, want %lu\n", c->name, got, c->want);
        else
            printf("ivsr_scale_ring_wrap: FAIL %s: got %lu, want %lu (the other ring gives %lu)\n",
                   c->name, got, c->want, c->other);
        return 1;
    }
    return 0;
}

/* Marker values are distinct per slot and the totals are derived from them, so
 * a marker edit cannot leave a stale expectation behind.  (The previous
 * revision hard-coded a linear total of 264 for markers summing to 209: its own
 * failure diagnostic named a value no execution could produce.) */
#define A0 11u
#define A1 22u
#define A2 33u
#define A3 44u
#define AL1 55u
#define AL2 66u
#define AL3 77u

/* scale 2: wrapped {0xFFFFFFFE, 0, 2, 4}, linear {0xFFFFFFFE, 2^32, +2, +4} */
static const struct mark marks_mul2[] = {
    {0xFFFFFFFEull, A0}, {0u, A1},       {2u, A2},        {4u, A3},
    {0x100000000ull, AL1}, {0x100000002ull, AL2}, {0x100000004ull, AL3},
};

#define B0 13u
#define B1 17u
#define B2 19u
#define B3 23u
#define BL1 29u
#define BL2 31u
#define BL3 37u

/* scale 4: wrapped {0xFFFFFFFC, 0, 4, 8}, linear {0xFFFFFFFC, 2^32, +4, +8} */
static const struct mark marks_mul4[] = {
    {0xFFFFFFFCull, B0}, {0u, B1},       {4u, B2},        {8u, B3},
    {0x100000000ull, BL1}, {0x100000004ull, BL2}, {0x100000008ull, BL3},
};

#define C0 41u
#define C1 43u
#define C2 47u
#define C3 53u
#define CL1 59u
#define CL2 61u
#define CL3 67u

/* add(iv,1): wrapped {0xFFFFFFFF, 0, 1, 2}, linear {0xFFFFFFFF, 2^32, +1, +2} */
static const struct mark marks_add[] = {
    {0xFFFFFFFFull, C0}, {0u, C1},       {1u, C2},        {2u, C3},
    {0x100000000ull, CL1}, {0x100000001ull, CL2}, {0x100000002ull, CL3},
};

#define D0 71u
#define D1 73u
#define D2 79u
#define D3 83u

/* sub(iv,1): wrapped {0, 0xFFFFFFFF, 0xFFFFFFFE, 0xFFFFFFFD}.  The linear
 * spelling of iteration 2 is buf-1, outside the mapping: a peeled displacement
 * faults, so there is no linear total to name. */
static const struct mark marks_sub[] = {
    {0u, D0}, {0xFFFFFFFFull, D1}, {0xFFFFFFFEull, D2}, {0xFFFFFFFDull, D3},
};

#define E0 89u
#define E1 97u

/* masked control: (i & 1) * 4 alternates between offsets 4 and 0, so the four
 * iterations read E1, E0, E1, E0.  Its own marker set -- sharing marks_mul2
 * would tie this expectation to another case's slots. */
static const struct mark marks_masked[] = {{0u, E0}, {4u, E1}};

int main(void) {
    static const struct case_def cases[] = {
        {"mul2-opaque", w_mul2_opaque, marks_mul2,
         sizeof marks_mul2 / sizeof marks_mul2[0], A0 + A1 + A2 + A3, A0 + AL1 + AL2 + AL3},
        {"mul2-const", w_mul2_const, marks_mul2,
         sizeof marks_mul2 / sizeof marks_mul2[0], A0 + A1 + A2 + A3, A0 + AL1 + AL2 + AL3},
        {"mul4-opaque", w_mul4_opaque, marks_mul4,
         sizeof marks_mul4 / sizeof marks_mul4[0], B0 + B1 + B2 + B3, B0 + BL1 + BL2 + BL3},
        {"addconst-opaque", w_addconst_opaque, marks_add,
         sizeof marks_add / sizeof marks_add[0], C0 + C1 + C2 + C3, C0 + CL1 + CL2 + CL3},
        /* CONTROLS: the linear total is the CORRECT one for these three. */
        {"struct-CONTROL", c_struct_element, marks_mul2,
         sizeof marks_mul2 / sizeof marks_mul2[0], A0 + AL1 + AL2 + AL3, A0 + A1 + A2 + A3},
        {"u64-CONTROL", c_u64_index, marks_mul2,
         sizeof marks_mul2 / sizeof marks_mul2[0], A0 + AL1 + AL2 + AL3, A0 + A1 + A2 + A3},
        {"masked-CONTROL", c_masked_index, marks_masked,
         sizeof marks_masked / sizeof marks_masked[0], 2u * (E0 + E1), 2u * (E0 + E1)},
        /* The faulting case runs last so every other verdict is reported
         * before a wrong compiler takes the process down. */
        {"subconst-opaque", w_subconst_opaque, marks_sub,
         sizeof marks_sub / sizeof marks_sub[0], D0 + D1 + D2 + D3, D0 + D1 + D2 + D3},
    };
    int bad = 0;
    for (unsigned i = 0; i < sizeof cases / sizeof cases[0]; i++) {
        int r = run_case(&cases[i]);
        if (r < 0) return 1;
        bad |= r;
    }
    if (bad) return 5;
    puts("ivsr_scale_ring_wrap: OK");
    return 0;
}
#endif
