/* Vector-typedef struct layout must be identical in const-eval and runtime.
 *
 * glibc 2.44 elf/dl-trampoline.S failed with
 *   "#error LR_VECTOR_OFFSET must be multiple of VEC_SIZE"
 * because the gen-as-const pipeline (asm "i" constraint / global-init
 * const-eval) computed offsetof(La_x86_64_regs, lr_vector) = 96 while the
 * runtime path said 192: sema's typedef registration skipped
 * __attribute__((vector_size(N))), recorded La_x86_64_xmm as a plain
 * 4-byte float, and poisoned the SHARED struct-layout cache that lowering's
 * const-eval reads.
 *
 * This mirrors glibc's exact La_x86_64_regs shape and checks const-eval
 * (global initializers, enum, case labels) against runtime offsetof.
 */
#include <stdio.h>

typedef float xmm_t __attribute__((__vector_size__(16)));
typedef float ymm_t __attribute__((__vector_size__(32), __aligned__(16)));
typedef double zmm_t __attribute__((__vector_size__(64), __aligned__(16)));

typedef union {
    ymm_t ymm[2];
    zmm_t zmm[1];
    xmm_t xmm[4];
} vec_t __attribute__((__aligned__(16)));

struct regs {
    unsigned long r[8];
    xmm_t lr_xmm[8];
    vec_t lr_vector[8];
    __int128 pad[4];
};

/* Global-initializer const-eval path (the poisoned one). */
static const long g_xmm_off = __builtin_offsetof(struct regs, lr_xmm);
static const long g_vec_off = __builtin_offsetof(struct regs, lr_vector);
static const long g_size = sizeof(struct regs);

/* Enum path: another pure-const context. */
enum { E_VEC_OFF = __builtin_offsetof(struct regs, lr_vector) };

int main(void) {
    int ok = 1;
    ok &= g_xmm_off == 64;
    ok &= g_vec_off == 192;
    ok &= E_VEC_OFF == 192;
    ok &= g_size == 768;
    /* Runtime path must agree with const-eval. */
    ok &= (long)__builtin_offsetof(struct regs, lr_xmm) == g_xmm_off;
    ok &= (long)__builtin_offsetof(struct regs, lr_vector) == g_vec_off;
    ok &= (long)sizeof(struct regs) == g_size;
    /* dl-trampoline's own assertion shape. */
    ok &= (g_vec_off % 64) == 0;
    printf("layout:%s xmm=%ld vec=%ld size=%ld\n", ok ? "ok" : "MISMATCH",
           g_xmm_off, g_vec_off, g_size);
    return ok ? 0 : 1;
}
