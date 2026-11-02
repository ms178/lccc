/*
 * Regression: x86-64 parameter pre-store ordering and fixed-GPR scratch model.
 *
 * Two defects (2026-09, at1/at2/at3/tsc1 corpus), both silent at -O1+:
 *
 * (1) LATE ABI READ AFTER PRE-STORE.  A register parameter whose ParamRef was
 *     left unhomed (e.g. the pointer operand of an atomic, which
 *     `remove_ineligible_operands` strips from allocation) and that has no
 *     alloca slot is read from its *incoming ABI register at the ParamRef
 *     site*.  The prologue's parallel copy had already moved ANOTHER
 *     parameter into that register (`movq %rcx, %rdi` for param 3 homed in
 *     %rdi), so the pointer was read as a scalar -> SIGSEGV.  The prologue
 *     now materialises every such late read at entry, before any ABI
 *     register is written.
 *
 * (2) FIXED-GPR SCRATCH.  The cmpxchg-loop RMWs (`fetch_and/or/xor/sub/nand`)
 *     use %rdx (new value) and %rdi (operand) as fixed scratch; `cmpxchg`
 *     uses %rdx for `desired`; atomic stores stage through %rdx; `rdtsc`
 *     writes %rdx, `rdtscp` also %rdi.  None of these were in the register
 *     census / caller-home gate, so allocator-owned values homed in %rdx or
 *     %rdi were destroyed mid-function.  `regalloc::x86_inst_fixed_scratch`
 *     is now the single model consumed by both the prologue census and the
 *     parameter-home gate.
 *
 * Every function is `noinline` with 5-6 register parameters so the homes are
 * ABI-decided and under pressure; results are compared against GCC.
 */
#include <stdio.h>
#include <stdint.h>
#include <x86intrin.h>

/* (1)+(2): pointer param 0 unhomed, params 1..5 in caller-saved homes, four
 * cmpxchg-loop RMWs clobbering rdx/rdi while temps t1..t8 are live. */
__attribute__((noinline)) long rmw_mix(long *p, long a, long b, long c, long d, long e) {
    long t1 = a * 3 + 1, t2 = b * 5 + 2, t3 = c * 7 + 3, t4 = d * 11 + 4, t5 = e * 13 + 5;
    long t6 = a ^ b, t7 = c ^ d, t8 = e ^ a;
    long old = __atomic_fetch_add(p, t1, __ATOMIC_SEQ_CST);
    long old2 = __atomic_fetch_and(p, t2 | 0x7f, __ATOMIC_SEQ_CST);
    long old3 = __atomic_fetch_or(p, t3, __ATOMIC_SEQ_CST);
    long old4 = __atomic_fetch_xor(p, t4, __ATOMIC_SEQ_CST);
    long old5 = __atomic_fetch_sub(p, t5 & 7, __ATOMIC_SEQ_CST);
    long old6 = __atomic_fetch_nand(p, t6 | 1, __ATOMIC_SEQ_CST);
    return old + old2 + old3 + old4 + old5 + old6 + t1 + t2 + t3 + t4 + t5 + t6 + t7 + t8 + *p;
}

/* cmpxchg: desired staged in rdx, expected in rax; param homes must survive. */
__attribute__((noinline)) long cas_mix(long *p, long a, long b, long c, long d, long e) {
    long t1 = a * 3 + 1, t2 = b * 5 + 2, t3 = c * 7 + 3, t4 = d * 11 + 4, t5 = e * 13 + 5;
    long exp = 100;
    int ok = __atomic_compare_exchange_n(p, &exp, t1, 0, __ATOMIC_SEQ_CST, __ATOMIC_SEQ_CST);
    long exp2 = t1;
    int ok2 = __atomic_compare_exchange_n(p, &exp2, t2 + t3, 0, __ATOMIC_SEQ_CST, __ATOMIC_SEQ_CST);
    long exp3 = -1;
    int ok3 = __atomic_compare_exchange_n(p, &exp3, t4, 0, __ATOMIC_SEQ_CST, __ATOMIC_SEQ_CST);
    return ok + ok2 * 2 + ok3 * 4 + t1 + t2 + t3 + t4 + t5 + exp + exp2 + exp3 + *p;
}

/* Narrow widths: the cmpxchg loop uses dil/di/edi and dl/dx/edx forms. */
__attribute__((noinline)) int rmw_narrow(unsigned char *pc, unsigned short *ps, unsigned *pi,
                                         int a, int b, int c) {
    int t1 = a * 3, t2 = b * 5, t3 = c * 7;
    unsigned char oc = __atomic_fetch_or(pc, (unsigned char)(t1 & 0x1f), __ATOMIC_RELAXED);
    unsigned short os = __atomic_fetch_xor(ps, (unsigned short)(t2 & 0xfff), __ATOMIC_RELAXED);
    unsigned oi = __atomic_fetch_and(pi, (unsigned)(t3 | 0xffff), __ATOMIC_RELAXED);
    return oc + os + (int)oi + t1 + t2 + t3 + *pc + *ps + (int)*pi;
}

/* Atomic load/store: pointer param unhomed, store stages value via rdx. */
__attribute__((noinline)) long ld_st(long *p, long a, long b, long c, long d, long e) {
    long q = __atomic_load_n(p, __ATOMIC_SEQ_CST);
    __atomic_store_n(p, q + a * 3, __ATOMIC_SEQ_CST);
    long q2 = __atomic_load_n(p, __ATOMIC_ACQUIRE);
    return q + q2 + a * 3 + b * 5 + c * 7 + d * 11 + e * 13;
}

/* Pointer in the LAST argument register(s): home permutations differ. */
__attribute__((noinline)) long ptr_last(long a, long b, long c, long d, long e, long *p) {
    long t1 = a * b, t2 = c * d, t3 = e * a;
    long old = __atomic_fetch_or(p, t1 & 0xff, __ATOMIC_SEQ_CST);
    return old + t1 + t2 + t3 + *p + b + c + d;
}

/* rdtsc/rdtscp: rdx (and rdi/rcx for rdtscp) are architectural outputs; the
 * live temps must not be homed there. The tick values are discarded. */
__attribute__((noinline)) long tsc_mix(long a, long b, long c, long d, long e) {
    long t1 = a * 3, t2 = b * 5, t3 = c * 7, t4 = d * 11, t5 = e * 13;
    unsigned long long t = __rdtsc();
    unsigned aux;
    unsigned long long t2c = __rdtscp(&aux);
    (void)t;
    (void)t2c;
    (void)aux;
    return t1 + t2 + t3 + t4 + t5 + a + b + c + d + e;
}

/* Two pointer params both unhomed + scalar params in caller-saved homes. */
__attribute__((noinline)) long two_ptrs(long *p, long *q, long a, long b, long c, long d) {
    long t1 = a * 3, t2 = b * 5, t3 = c * 7, t4 = d * 11;
    long o1 = __atomic_fetch_xor(p, t1, __ATOMIC_SEQ_CST);
    long o2 = __atomic_fetch_and(q, ~t2, __ATOMIC_SEQ_CST);
    return o1 + o2 + t1 + t2 + t3 + t4 + *p + *q;
}

/* Narrow ParamRef types (int/short/char) read late from the ABI register:
 * the typed early materialisation must use the same movslq/movswq/movsbq
 * forms as the in-body read. */
__attribute__((noinline)) long narrow_params(long *p, int a, short b, signed char c, unsigned d,
                                             unsigned short e) {
    long o = __atomic_fetch_or(p, (long)a, __ATOMIC_SEQ_CST);
    return o + a * 3 + b * 5 + c * 7 + (long)d * 11 + e * 13 + *p;
}

int main(void) {
    long x = 100;
    long r1 = rmw_mix(&x, 1, 2, 3, 4, 5);
    long x1 = x;
    x = 100;
    long r2 = cas_mix(&x, 1, 2, 3, 4, 5);
    long x2 = x;
    unsigned char pc = 0x40;
    unsigned short ps = 0x1234;
    unsigned pi = 0xdeadbeef;
    int r3 = rmw_narrow(&pc, &ps, &pi, 9, 8, 7);
    x = 100;
    long r4 = ld_st(&x, 1, 2, 3, 4, 5);
    long x4 = x;
    x = 100;
    long r5 = ptr_last(1, 2, 3, 4, 5, &x);
    long x5 = x;
    long r6 = tsc_mix(1, 2, 3, 4, 5);
    long y = 77;
    x = 100;
    long r7 = two_ptrs(&x, &y, 1, 2, 3, 4);
    long x7 = x, y7 = y;
    x = 100;
    long r8 = narrow_params(&x, -3, -5, -7, 11u, 13u);
    long x8 = x;
    printf("%ld %ld\n", r1, x1);
    printf("%ld %ld\n", r2, x2);
    printf("%d %u %u %u\n", r3, pc, ps, pi);
    printf("%ld %ld\n", r4, x4);
    printf("%ld %ld\n", r5, x5);
    printf("%ld\n", r6);
    printf("%ld %ld %ld\n", r7, x7, y7);
    printf("%ld %ld\n", r8, x8);
    return 0;
}
