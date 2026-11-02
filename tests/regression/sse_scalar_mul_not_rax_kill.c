/* PEEP-MULSS-RAX-1: a legacy-SSE `mulss`/`divsd` is not an implicit %rax
 * write.
 *
 * The peephole's destination classifier (`parse_dest_reg_fast`) prefix-
 * matched every mnemonic starting with `mul`/`div` as the one-operand integer
 * form (implicit %rax:%rdx), so the CFG liveness saw `mulss %xmm2, %xmm3` as
 * a KILL of %rax.  An address computed into %rax before the multiply and
 * dereferenced after it was therefore "dead" at its `addq`/`andq`, and
 * dead-flag-write elimination deleted the realignment of this 64-byte-aligned
 * vector:
 *
 *   leaq 16(%rsp), %rax ; addq $63, %rax ; andq $-64, %rax   <- both deleted
 *   pxor %xmm3, %xmm3 ; cvtsi2ssq %r11, %xmm3 ; mulss %xmm2, %xmm3
 *   movss %xmm3, (%r10, %rax)                                <- wrong address
 *
 * Only the VEX-less code path (-mno-avx, x86-64-v2 targets) spells the
 * multiply `mulss`; the default v3 baseline emits `vmulss`, which the
 * classifier never matched -- so the bug hid behind the default flags. */
#include <stdio.h>

typedef float v16f __attribute__((vector_size(64)));
typedef double v4d __attribute__((vector_size(32)));

__attribute__((noinline)) float sum16(const float *p) {
    float s = 0;
    for (int i = 0; i < 16; i++) s += p[i] * (float)(i + 1);
    return s;
}

__attribute__((noinline)) double sum4(const double *p) {
    double s = 0;
    for (int i = 0; i < 4; i++) s += p[i] * (double)(i + 1);
    return s;
}

__attribute__((noinline)) float fill_mul(float k) {
    v16f wa;
    for (int i = 0; i < 16; i++) wa[i] = (float)i * k;
    return sum16((const float *)&wa);
}

__attribute__((noinline)) double fill_div(double k) {
    v4d d;
    for (int i = 0; i < 4; i++) d[i] = (double)(i + 1) / k;
    return sum4((const double *)&d);
}

int main(void) {
    printf("%.3f\n", fill_mul(1.5f));
    printf("%.6f\n", fill_div(4.0));
    return 0;
}
