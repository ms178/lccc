/* AFFOLD regression corpus — the default-on affine exit-compare fold.
 *
 * The fold rewrites `iv + C <op> N` to `iv <op> N - C` (signed `<op>` only)
 * for every natural loop, with a proof obligation that the sequence
 * `iv, iv + step, ...` cannot wrap across the tests the loop actually
 * performs.  A wrong fold shows up first and most sharply as a wrong ITERATION
 * COUNT, so every kernel in this file reports the count it ran as well as its
 * checksum: `check_affine_loop_fold.sh` compiles this file with the GCC oracle
 * and with lccc under every configuration (fold on/off, rotation on/off,
 * -O1/-O2/-O3/-Os) and requires byte-identical stdout.
 *
 * What the shapes are for:
 *
 *   sweep_*        constant (C, N, start, step) grid, 32- and 64-bit IVs —
 *                  the fold's normal operating range; iteration counts are
 *                  exact, so an off-by-`step` fold cannot hide.
 *   near-high/low  the representability boundaries: a 64-bit loop whose
 *                  *failing* test computes exactly LLONG_MAX (last reachable
 *                  test at the top of the type), and one at the bottom of the
 *                  type.  Both are well-defined C and both are accepted by the
 *                  fold only because it checks both ends of the sequence.
 *   top_end_*      start so close to the type top that the *entry* test cannot
 *                  be proven representable: the fold must refuse (0 iterations
 *                  either way, so contract 3 pins the refusal in the asm).
 *   refuse_*       shapes the fold must not touch but must keep correct:
 *                  runtime bound, runtime addend, runtime start, runtime step,
 *                  unsigned compare, 32-bit compare against a 64-bit addend
 *                  (Add width != Cmp width), and a temporary with two uses
 *                  (folding would not remove the add, so it is not profitable).
 *   nested_*       the reason the fold is its own pass: rotation's Guard E
 *                  refuses nested loops, and every filter kernel's inner loop
 *                  is nested.  Literal constants, so the fold applies.
 *   multiblock      body with a branch in it: the affine temporary and the
 *                  compare may live in different blocks of the loop body.
 *   inner_cmp       an affine compare that is NOT the exit test: the rewrite is
 *                  a value equivalence, not a loop-shape transform.
 *   ptr_iv          `p + 4 < v + N` — pointer IVs are GEPs, not affine adds.
 *
 * All loads are masked into `buf`, so the same file can be reused with any
 * bound constant; `volatile` reads keep the kernels from being constant-folded
 * away entirely.
 */
#include <limits.h>
#include <stdio.h>

static unsigned char buf[4096];

static unsigned long h = 1469598103934665603UL;
static void mix(long tag, long cnt, long sum) {
    h = (h ^ (unsigned long)(tag * 2654435761U + (unsigned long)cnt)) * 1099511628211UL;
    h = (h ^ (unsigned long)sum) * 1099511628211UL;
}

/* ------------------------------------------------------------------ sweep */
#define KERNEL(NAME, TY, C, N, START, STEP)                      \
    static void NAME(long *cnt, long *sum) {                     \
        TY s = 0;                                                \
        long n = 0;                                              \
        for (TY i = (START); i + (C) < (TY)(N); i += (STEP)) {   \
            s += buf[(long)(i) & 4095];                          \
            n++;                                                 \
        }                                                        \
        *cnt = n;                                                \
        *sum = (long)s;                                          \
    }

KERNEL(k_c1_n64, int, 1, 64, 0, 1)
KERNEL(k_c4_n64, int, 4, 64, 0, 1)
KERNEL(k_c7_n64, int, 7, 64, 0, 1)
KERNEL(k_c4_start3, int, 4, 64, 3, 1)
KERNEL(k_c4_step3, int, 4, 64, 0, 3)
KERNEL(k_c8_step5, int, 8, 64, 0, 5)
KERNEL(k_c0_n64, int, 0, 64, 0, 1)
KERNEL(k_c4_n2, int, 4, 2, 0, 1)
KERNEL(k_c4_n5, int, 4, 5, 0, 1)
KERNEL(k_c4_n100_s95, int, 4, 100, 95, 1)
KERNEL(k_c12_n100_s99, int, 12, 100, 99, 1)
KERNEL(l_c1_n4096, long long, 1, 4096, 0, 1)
KERNEL(l_c4_n4096, long long, 4, 4096, 0, 1)
KERNEL(l_c4_n8192_s1, long long, 4, 8192, 1, 1)
KERNEL(l_c1_n4097_step2, long long, 1, 4097, 0, 2)
KERNEL(l_c4_n4096_step3, long long, 4, 4096, 0, 3)

/* Flipped / non-`<` comparator spellings: `N > i + C`, `i + C <= N`, ... */
static void flipped_cmp(long *cnt, long *sum) {
    long s = 0;
    long n = 0;
    for (long long i = 0; 64 > i + 4; i++) {
        s += buf[i & 4095];
        n++;
    }
    *cnt = n;
    *sum = s;
}

static void le_cmp(long *cnt, long *sum) {
    long s = 0;
    long n = 0;
    for (long long i = 0; i + 4 <= 64; i++) {
        s += buf[i & 4095];
        n++;
    }
    *cnt = n;
    *sum = s;
}

static void const_left_add(long *cnt, long *sum) {
    long s = 0;
    long n = 0;
    for (long long i = 0; 4 + i < 64; i++) {
        s += buf[i & 4095];
        n++;
    }
    *cnt = n;
    *sum = s;
}

/* ------------------------------------------------- representability edges */
static void near_high(long *cnt, long *sum) {
    /* The failing test computes LLONG_MAX + 0 -- exactly representable, so
       this loop is well-defined and the fold's last-test check accepts it. */
    long s = 0;
    long n = 0;
    for (long long i = LLONG_MAX - 20; i + 4 < LLONG_MAX; i++) {
        s += buf[i & 4095];
        n++;
    }
    *cnt = n;
    *sum = s;
}

static void near_low(long *cnt, long *sum) {
    /* Same at the bottom of the type: the sequence starts at LLONG_MIN and
       the first test is `LLONG_MIN + 4`. */
    long s = 0;
    long n = 0;
    for (long long i = LLONG_MIN; i + 4 < (long long)(LLONG_MIN + 8); i++) {
        s += buf[i & 4095];
        n++;
    }
    *cnt = n;
    *sum = s;
}

static void top_end_start(long *cnt, long *sum) {
    /* Entry test `LLONG_MAX - 4 + 4` is representable, the *iteration* test
       after stepping would not be; the fold must not sign off on this one
       (first-test check), and either way the loop runs zero times. */
    long s = 0;
    long n = 0;
    for (long long i = LLONG_MAX - 4; i + 4 < LLONG_MAX; i++) {
        s += buf[i & 4095];
        n++;
    }
    *cnt = n;
    *sum = s;
}

static void high_one_iter(long *cnt, long *sum) {
    /* One iteration whose *failing* test is the last representable value:
       `start + step + C` and `new_bound + step - 1` both land exactly on
       LLONG_MAX.  This is the fold's last-test boundary in the accepted
       direction -- a program that steps once more would be UB, which is why
       the boundary cases that *violate* the check can only be 0-iteration
       shapes (pinned in the asm by contract 3). */
    long s = 0;
    long n = 0;
    for (long long i = LLONG_MAX - 5; i + 4 < LLONG_MAX; i++) {
        s += buf[i & 4095];
        n++;
    }
    *cnt = n;
    *sum = s;
}

/* --------------------------------------------------------------- refusals
   (the high/low boundary rows above are the *accepted* edge of each check;
   the rows below the ones the fold must refuse while staying exact) */
static void refuse_rt_bound(long *cnt, long *sum) {
    volatile long N = 64; /* runtime bound: not an affine constant */
    long s = 0;
    long n = 0;
    for (long long i = 0; i + 4 < N; i++) {
        s += buf[i & 4095];
        n++;
    }
    *cnt = n;
    *sum = s;
}

static void refuse_rt_addend(long *cnt, long *sum) {
    volatile long C = 4; /* runtime addend: the add is not `iv + Const` */
    long s = 0;
    long n = 0;
    for (long long i = 0; i + C < 64; i++) {
        s += buf[i & 4095];
        n++;
    }
    *cnt = n;
    *sum = s;
}

static void refuse_rt_start(long *cnt, long *sum) {
    volatile long S = 3; /* runtime seed: no constant entry value to prove */
    long s = 0;
    long n = 0;
    for (long long i = S; i + 4 < 64; i++) {
        s += buf[i & 4095];
        n++;
    }
    *cnt = n;
    *sum = s;
}

static void refuse_rt_step(long *cnt, long *sum) {
    volatile long P = 3; /* runtime step: the phi's back edge is not +Const */
    long s = 0;
    long n = 0;
    for (long long i = 0; i + 4 < 64; i += P) {
        s += buf[i & 4095];
        n++;
    }
    *cnt = n;
    *sum = s;
}

static void refuse_negative_c(long *cnt, long *sum) {
    long s = 0;
    long n = 0;
    for (long long i = 0; i - 4 < 64; i++) {
        if (i > 80) break; /* the shape is `i + (-4)`; bound the run */
        s += buf[i & 4095];
        n++;
    }
    *cnt = n;
    *sum = s;
}

static void refuse_unsigned(long *cnt, long *sum) {
    unsigned s = 0;
    long n = 0;
    for (unsigned i = 0; i + 4u < 64u; i++) {
        s += buf[i & 4095];
        n++;
    }
    *cnt = n;
    *sum = (long)s;
}

static void refuse_width_mismatch(long *cnt, long *sum) {
    /* 32-bit compare of a 64-bit add: the add's width is what makes the
       constant affine, so a narrow compare must not fold the wide add. */
    long long s = 0;
    long n = 0;
    for (long long i = 0; (int)(i + 4) < 64; i++) {
        if (i > 80) break;
        s += buf[i & 4095];
        n++;
    }
    *cnt = n;
    *sum = s;
}

static void refuse_two_uses(long *cnt, long *sum) {
    long s = 0;
    long n = 0;
    for (long long i = 0; i < 64; i++) {
        long long t = i + 4; /* two uses: the add survives, no profit */
        if (t < 4096) {
            s += buf[t & 4095];
            n++;
        }
    }
    *cnt = n;
    *sum = s;
}

static void ptr_iv(long *cnt, long *sum) {
    const unsigned char *v = buf;
    long s = 0;
    long n = 0;
    for (const unsigned char *p = v; p + 4 < v + 64; p++) {
        s += *p;
        n++;
    }
    *cnt = n;
    *sum = s;
}

/* --------------------------------------------------------------- nesting */
static void nested_two(long *cnt, long *sum) {
    long s = 0;
    long n = 0;
    for (int rep = 0; rep < 12; rep++) {
        for (long long i = 0; i + 4 < 4096; i++) {
            s += buf[i & 4095] + rep;
            n++;
        }
        buf[rep & 4095] ^= (unsigned char)rep;
    }
    *cnt = n;
    *sum = s;
}

static void nested_three(long *cnt, long *sum) {
    long s = 0;
    long n = 0;
    for (int a = 0; a < 4; a++) {
        for (int b = 0; b < 3; b++) {
            for (long long i = 0; i + 7 < 1024; i++) {
                s += buf[(i & 4095) ^ a] + b;
                n++;
            }
        }
    }
    *cnt = n;
    *sum = s;
}

static void nested_step(long *cnt, long *sum) {
    long s = 0;
    long n = 0;
    for (int rep = 0; rep < 8; rep++) {
        for (long long i = 1; i + 8 < 4097; i += 5) {
            s += buf[i & 4095];
            n++;
        }
    }
    *cnt = n;
    *sum = s;
}

/* -------------------------------------------------------------- self-loop
   A do-while body with no branches is ONE block: the phi, its increment and
   the exit test all live there and the block branches to itself.  The fold
   classifies the phi's incomings by value (the increment is the incoming that
   is `phi + Const`, the other must be a constant seed), so these fold even
   though the "entry" edge's predecessor is the header itself. */
static void selfloop(long *cnt, long *sum) {
    long s = 0;
    long n = 0;
    long long i = 0;
    do {
        s += buf[i & 4095];
        n++;
        i++;
    } while (i + 4 < 4096);
    *cnt = n;
    *sum = s;
}

static void while_guard(long *cnt, long *sum) {
    long s = 0;
    long n = 0;
    long long i = 0;
    while (i + 4 < 2048) {
        s += buf[i & 4095];
        n++;
        i++;
    }
    *cnt = n;
    *sum = s;
}

/* ------------------------------------------------------------- multiblock */
static void multiblock(long *cnt, long *sum) {
    long s = 0;
    long n = 0;
    for (long long i = 0; i + 4 < 8192; i++) {
        if (i & 1) {
            s += buf[i & 4095] + 1;
        } else {
            s -= buf[(i + 1) & 4095];
        }
        n++;
    }
    *cnt = n;
    *sum = s;
}

static void inner_cmp(long *cnt, long *sum) {
    /* The affine compare is a body compare, not the exit test. */
    long s = 0;
    long n = 0;
    for (long long i = 0; i < 4096; i++) {
        if (i + 4 < 100) {
            s += buf[i & 4095];
            n++;
        }
    }
    *cnt = n;
    *sum = s;
}

int main(void) {
    struct {
        const char *name;
        void (*fn)(long *, long *);
    } kernels[] = {
        {"k_c1_n64", k_c1_n64},
        {"k_c4_n64", k_c4_n64},
        {"k_c7_n64", k_c7_n64},
        {"k_c4_start3", k_c4_start3},
        {"k_c4_step3", k_c4_step3},
        {"k_c8_step5", k_c8_step5},
        {"k_c0_n64", k_c0_n64},
        {"k_c4_n2", k_c4_n2},
        {"k_c4_n5", k_c4_n5},
        {"k_c4_n100_s95", k_c4_n100_s95},
        {"k_c12_n100_s99", k_c12_n100_s99},
        {"l_c1_n4096", l_c1_n4096},
        {"l_c4_n4096", l_c4_n4096},
        {"l_c4_n8192_s1", l_c4_n8192_s1},
        {"l_c1_n4097_step2", l_c1_n4097_step2},
        {"l_c4_n4096_step3", l_c4_n4096_step3},
        {"flipped_cmp", flipped_cmp},
        {"le_cmp", le_cmp},
        {"const_left_add", const_left_add},
        {"near_high", near_high},
        {"near_low", near_low},
        {"top_end_start", top_end_start},
        {"high_one_iter", high_one_iter},
        {"refuse_rt_bound", refuse_rt_bound},
        {"refuse_rt_addend", refuse_rt_addend},
        {"refuse_rt_start", refuse_rt_start},
        {"refuse_rt_step", refuse_rt_step},
        {"refuse_negative_c", refuse_negative_c},
        {"refuse_unsigned", refuse_unsigned},
        {"refuse_width_mismatch", refuse_width_mismatch},
        {"refuse_two_uses", refuse_two_uses},
        {"ptr_iv", ptr_iv},
        {"nested_two", nested_two},
        {"nested_three", nested_three},
        {"nested_step", nested_step},
        {"selfloop", selfloop},
        {"while_guard", while_guard},
        {"multiblock", multiblock},
        {"inner_cmp", inner_cmp},
    };
    for (unsigned k = 0; k < sizeof(kernels) / sizeof(kernels[0]); k++) {
        buf[k & 4095] = (unsigned char)(k * 37 + 11);
        long cnt = -1, sum = 0;
        kernels[k].fn(&cnt, &sum);
        mix((long)k, cnt, sum);
        printf("%-20s cnt=%-8ld sum=%ld\n", kernels[k].name, cnt, sum);
    }
    printf("affine-loop-fold: all shapes exact (hash %lx)\n", h);
    return 0;
}
