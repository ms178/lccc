/* Loop-unroll red-team corpus. Every kernel is noinline so the shape the
 * unroller sees is the shape written here; runtime trip counts come from
 * volatile globals so no earlier pass can fold them. Output is a stream of
 * "name=value" lines compared against GCC. */
#include <stdio.h>
#include <limits.h>
#include <string.h>
#define NOINLINE __attribute__((noinline))
volatile int vn4 = 4, vn5 = 5, vn7 = 7, vn8 = 8, vn9 = 9, vn17 = 17, vn0 = 0, vn1 = 1, vn3 = 3;
volatile int vflag1 = 1, vflag0 = 0;
volatile long vln = 13;
int A[64], B[64], C[64];
long LA[64];
unsigned char UA[64];
static void init(void) {
    for (int i = 0; i < 64; i++) { A[i] = i * 7 - 20; B[i] = (i * 13) % 11 - 5; C[i] = 0; LA[i] = i * 3L - 9; UA[i] = (unsigned char)(i * 37); }
}
/* 1. "first iteration" flag: carried phi whose back value is a CONSTANT. */
NOINLINE int k_first_const4(void) {
    int s = 0, first = 1;
    for (int i = 0; i < 4; i++) { s += first ? A[i] * 100 : A[i]; first = 0; }
    return s;
}
NOINLINE int k_first_const_rt(int n) {
    int s = 0, first = 1;
    for (int i = 0; i < n; i++) { s += first ? A[i] * 100 : A[i]; first = 0; }
    return s;
}
/* 2. carried phi whose back value is a loop-INVARIANT parameter. */
NOINLINE int k_first_param4(int p) {
    int s = 0, cur = 1;
    for (int i = 0; i < 4; i++) { s += cur * A[i]; cur = p; }
    return s;
}
NOINLINE int k_first_param_rt(int p, int n) {
    int s = 0, cur = 1;
    for (int i = 0; i < n; i++) { s += cur * A[i]; cur = p; }
    return s;
}
/* 3. Second IV updated in the latch (comma increment). */
NOINLINE int k_two_iv_rt(int n) {
    int s = 0;
    int j;
    int i;
    for (i = 0, j = n - 1; i < n; i++, j--) s += A[i] * B[j];
    return s + j;
}
NOINLINE int k_ptr_iv_rt(int n) {
    int s = 0; int *p = A; int *q = B + 20;
    for (int i = 0; i < n; i++, p++, q--) s += *p - *q;
    return s + (int)(p - A) + (int)(q - B);
}
NOINLINE int k_two_iv4(void) {
    int s = 0; int j;
    for (int i = 0, j2 = 3; i < 4; i++, j2--) { s += A[i] * B[j2]; j = j2; }
    return s + j;
}
/* 4. Diamond with phi inside the body (stores make select-conversion illegal). */
NOINLINE int k_diamond_rt(int n) {
    int s = 0;
    for (int i = 0; i < n; i++) {
        int t;
        if (A[i] < 0) { C[i] = 1; t = -A[i]; } else { C[i + 1] += 2; t = A[i] * 3; }
        s += t;
    }
    return s;
}
NOINLINE int k_diamond4(void) {
    int s = 0;
    for (int i = 0; i < 4; i++) {
        int t;
        if (B[i] < 0) { C[i + 8] = 1; t = -B[i]; } else { C[i + 9] += 2; t = B[i] * 3; }
        s += t;
    }
    return s;
}
/* 5. Unsigned constants that are negative when sign-extended. */
NOINLINE int k_u_zero_trip_a(void) { int c = 0; for (unsigned i = 0xFFFFFFFEu; i < 2u; i++) c++; return c; }
NOINLINE int k_u_zero_trip_b(void) { int c = 0; for (unsigned i = 3u; i > 0xFFFFFFFEu; i--) c++; return c; }
NOINLINE int k_u_zero_trip_c(void) { int c = 0; for (unsigned i = -3u; i < 5u; i++) c++; return c; }
NOINLINE int k_u_zero_trip_d(void) { int c = 0; for (unsigned i = 1u; i <= 0xFFFFFFFDu; i += 0x7FFFFFFFu) c++; return c; }
NOINLINE unsigned k_u_hi_trip(void) { unsigned s = 0; for (unsigned i = 0xFFFFFFF0u; i < 0xFFFFFFF4u; i++) s += i & 15; return s; }
NOINLINE unsigned k_u_hi_trip_le(void) { unsigned s = 0; for (unsigned i = 0xFFFFFFFCu; i <= 0xFFFFFFFEu; i++) s += i & 15; return s; }
NOINLINE unsigned k_u_down_hi(void) { unsigned s = 0; for (unsigned i = 0xFFFFFFFFu; i > 0xFFFFFFFBu; i--) s += i & 15; return s; }
NOINLINE unsigned long k_ul_hi(void) { unsigned long s = 0; for (unsigned long i = ~0ul - 3; i < ~0ul; i++) s += i & 15; return s; }
NOINLINE long k_l_max(void) { long s = 0; for (long i = LONG_MAX - 3; i < LONG_MAX; i++) s += i & 15; return s; }
NOINLINE long k_l_min(void) { long s = 0; for (long i = LONG_MIN + 3; i > LONG_MIN; i--) s += i & 15; return s; }
/* 6. Comparison written the other way round / negated / != */
NOINLINE int k_rev_gt(void) { int s = 0; for (int i = 0; 4 > i; i++) s += A[i]; return s; }
NOINLINE int k_rev_ge(void) { int s = 0; for (int i = 0; 4 >= i; i++) s += A[i]; return s; }
NOINLINE int k_neg_ge(void) { int s = 0; for (int i = 0; !(i >= 4); i++) s += A[i]; return s; }
NOINLINE int k_ne(void) { int s = 0; for (int i = 0; i != 4; i++) s += A[i]; return s; }
NOINLINE int k_ne_step3(void) { int s = 0; for (int i = 0; i != 12; i += 3) s += A[i]; return s; }
NOINLINE int k_zero_ge(void) { int s = 0; for (int i = 0; i >= 4; i++) s += A[i]; return s; }
NOINLINE int k_rev_lt_exit(void) { int s = 0; for (int i = 0; 4 < i ? 0 : 1; i++) s += A[i]; return s; }
NOINLINE int k_le_neg_init(void) { int s = 0; for (int i = -3; i <= 3; i += 2) s += A[i + 3]; return s; }
NOINLINE int k_down_ge(void) { int s = 0; for (int i = 9; i >= 0; i -= 2) s += A[i]; return s; }
NOINLINE int k_down_gt_sub(void) { int s = 0; for (int i = 10; i > 0; i -= 3) s += A[i]; return s; }
/* 7. Side effect in the loop condition (header block). */
NOINLINE int k_header_effect(void) {
    int cnt = 0, s = 0;
    for (int i = 0; cnt++, i < 4; i++) s += A[i];
    return s * 100 + cnt;
}
NOINLINE int k_header_store(void) {
    int s = 0;
    for (int i = 0; (C[16 + i] = i * 5), i < 4; i++) s += A[i];
    return s + C[16] + C[17] + C[18] + C[19] + C[20];
}
/* 8. Exit block is a merge with a phi (loop on one arm). */
NOINLINE int k_exit_merge(int flag) {
    int r = 0;
    if (flag) { for (int i = 0; i < 4; i++) r += A[i]; } else { r = 7; }
    return r;
}
NOINLINE int k_exit_merge_rt(int flag, int n) {
    int r = 0;
    if (flag) { for (int i = 0; i < n; i++) r += A[i]; } else { r = 7; }
    return r;
}
NOINLINE int k_exit_iv_merge(int flag, int n) {
    int i = 100;
    if (flag) { for (i = 0; i < n; i++) C[32 + i] = i; }
    return i;
}
/* 9. Values live-out of a partially unrolled loop: IV, accumulator, last element. */
NOINLINE int k_liveout_rt(int n) {
    int i, s = 0, last = -1;
    for (i = 0; i < n; i++) { s += A[i]; last = A[i]; }
    return s * 1000 + i * 10 + last;
}
/* 10. Narrow IVs. */
NOINLINE int k_uchar_ne(void) { int c = 0; for (unsigned char i = 250; i != 2; i++) c += i; return c; }
NOINLINE int k_short_neg(void) { int c = 0; for (short i = -3; i < 3; i++) c += i * i; return c; }
NOINLINE int k_schar_edge(void) { int c = 0; for (signed char i = 124; i < 127; i++) c += i; return c; }
NOINLINE int k_uchar_hi(void) { int c = 0; for (unsigned char i = 253; i < 255; i++) c += i; return c; }
/* 11. continue inside body (latch has several predecessors). */
NOINLINE int k_continue_rt(int n) {
    int s = 0;
    for (int i = 0; i < n; i++) { if (A[i] & 1) continue; s += A[i]; }
    return s;
}
/* 12. Loop-carried phi with the latch listed first in the phi (order stress). */
NOINLINE int k_stride_carried(void) {
    int x = 1, y = 2;
    for (int i = 0; i < 8; i += 2) { int t = x + y; x = y; y = t + A[i]; }
    return x * 31 + y;
}
/* 13. Nested loops: constant inner, runtime outer and vice versa. */
NOINLINE int k_nested_rt_outer(int n) {
    int s = 0;
    for (int i = 0; i < n; i++) for (int j = 0; j < 4; j++) s += A[i + j] * (j + 1);
    return s;
}
NOINLINE int k_nested_rt_inner(int n) {
    int s = 0;
    for (int i = 0; i < 4; i++) for (int j = 0; j < n; j++) s += A[i + j] * (i + 1);
    return s;
}
NOINLINE int k_triangular(void) {
    int s = 0;
    for (int i = 0; i < 5; i++) for (int j = i + 1; j < 5; j++) s += A[i] * B[j];
    return s;
}
/* 14. FP reductions with constant/runtime trips. */
NOINLINE double k_f64_rt(int n) { double s = 0; for (int i = 0; i < n; i++) s += LA[i] * 0.5; return s; }
NOINLINE float k_f32_c6(void) { float s = 1; for (int i = 0; i < 6; i++) s = s * 0.5f + A[i]; return s; }
/* 15. Runtime trips around the unroll factor: 0,1,3,4,5,7,8,9,17. */
NOINLINE int k_sum_rt(int n) { int s = 0; for (int i = 0; i < n; i++) s += A[i] ^ i; return s; }
NOINLINE int k_sum_le_rt(int n) { int s = 0; for (int i = 0; i <= n; i++) s += A[i] ^ i; return s; }
NOINLINE int k_down_rt(int n) { int s = 0; for (int i = n; i > 0; i--) s += A[i] * i; return s; }
NOINLINE int k_stride3_rt(int n) { int s = 0; for (int i = 0; i < n; i += 3) s += A[i]; return s; }
NOINLINE unsigned k_u_rt(unsigned n) { unsigned s = 0; for (unsigned i = 0; i < n; i++) s += UA[i]; return s; }
NOINLINE long k_l_rt(long n) { long s = 0; for (long i = 0; i < n; i++) s += LA[i]; return s; }
/* 16. Multiple carried values + IV used in address arithmetic. */
NOINLINE int k_multi_carried_rt(int n) {
    int a = 1, b = 2, c = 3;
    for (int i = 0; i < n; i++) { int t = a; a = b + A[i]; b = c ^ t; c = t + 1; }
    return a * 7 + b * 3 + c;
}
/* 17. Store-then-load dependence through memory in unrolled body. */
NOINLINE int k_mem_dep_rt(int n) {
    C[40] = 1;
    for (int i = 0; i < n; i++) C[41 + i] = C[40 + i] * 2 + 1;
    return C[40 + n];
}
NOINLINE int k_mem_dep4(void) {
    C[50] = 3;
    for (int i = 0; i < 4; i++) C[51 + i] = C[50 + i] + A[i];
    return C[54];
}
/* 18. Early exit from body (break) — must be left alone or handled. */
NOINLINE int k_break_rt(int n) {
    int i;
    for (i = 0; i < n; i++) if (A[i] > 20) break;
    return i;
}
/* 19. volatile in body */
NOINLINE int k_volatile4(void) {
    int s = 0;
    for (int i = 0; i < 4; i++) s += vn4 + i;
    return s;
}
/* 20. Bool-flag latch-first phi: x = 1 then x = x? param : 0 */
NOINLINE int k_flag_toggle_rt(int n, int p) {
    int x = 1, s = 0;
    for (int i = 0; i < n; i++) { s += x; x = x ? p : 0; }
    return s;
}
/* 21. IV compared with widening use of iv (i32 iv, i64 index) */
NOINLINE long k_widen_rt(int n) {
    long s = 0;
    for (int i = 0; i < n; i++) s += LA[i] * (long)i;
    return s;
}
/* 22. Complete unroll where the IV final value is used after. */
NOINLINE int k_iv_after8(void) { int i, s = 0; for (i = 3; i < 19; i += 2) s += A[i]; return s * 100 + i; }
/* 23. Signed init negative, unsigned compare (should be zero trips). */
NOINLINE int k_u_neg_init(void) { int c = 0; for (int i = -2; (unsigned)i < 3u; i++) c++; return c; }
/* 24. Step larger than span with <= (exactly one trip) and one-trip loops. */
NOINLINE int k_one_trip(void) { int s = 0; for (int i = 0; i <= 0; i++) s += A[i] + 5; return s; }
NOINLINE int k_big_step(void) { int s = 0; for (int i = 0; i < 100; i += 60) s += A[i % 64]; return s; }

int k_mixed_back(int p, int n);
int k_trip16(void);
int k_trip16_carried(void);
int k_desc_carried(void);
int k_live_mixed(int n);
long k_wide_inv(int n);
int main(void) {
    init();
    printf("first_const4=%d\n", k_first_const4());
    printf("first_const_rt5=%d rt9=%d rt1=%d rt0=%d\n", k_first_const_rt(vn5), k_first_const_rt(vn9), k_first_const_rt(vn1), k_first_const_rt(vn0));
    printf("first_param4=%d\n", k_first_param4(vn3));
    printf("first_param_rt=%d\n", k_first_param_rt(vn3, vn9));
    printf("two_iv_rt=%d %d %d\n", k_two_iv_rt(vn9), k_two_iv_rt(vn4), k_two_iv_rt(vn17));
    printf("ptr_iv_rt=%d %d\n", k_ptr_iv_rt(vn9), k_ptr_iv_rt(vn7));
    printf("two_iv4=%d\n", k_two_iv4());
    printf("diamond_rt=%d\n", k_diamond_rt(vn9));
    printf("diamond4=%d\n", k_diamond4());
    printf("C=");
    for (int i = 0; i < 64; i++) printf("%d,", C[i]);
    printf("\n");
    printf("u_zero=%d %d %d %d\n", k_u_zero_trip_a(), k_u_zero_trip_b(), k_u_zero_trip_c(), k_u_zero_trip_d());
    printf("u_hi=%u %u %u %lu %ld %ld\n", k_u_hi_trip(), k_u_hi_trip_le(), k_u_down_hi(), k_ul_hi(), k_l_max(), k_l_min());
    printf("cmp_forms=%d %d %d %d %d %d %d %d %d %d\n", k_rev_gt(), k_rev_ge(), k_neg_ge(), k_ne(), k_ne_step3(), k_zero_ge(), k_rev_lt_exit(), k_le_neg_init(), k_down_ge(), k_down_gt_sub());
    printf("header=%d %d\n", k_header_effect(), k_header_store());
    printf("exit_merge=%d %d %d %d %d %d\n", k_exit_merge(vflag1), k_exit_merge(vflag0), k_exit_merge_rt(vflag1, vn9), k_exit_merge_rt(vflag0, vn9), k_exit_iv_merge(vflag1, vn9), k_exit_iv_merge(vflag0, vn9));
    printf("liveout=%d %d %d %d\n", k_liveout_rt(vn9), k_liveout_rt(vn8), k_liveout_rt(vn1), k_liveout_rt(vn0));
    printf("narrow=%d %d %d %d\n", k_uchar_ne(), k_short_neg(), k_schar_edge(), k_uchar_hi());
    printf("continue=%d\n", k_continue_rt(vn17));
    printf("stride_carried=%d\n", k_stride_carried());
    printf("nested=%d %d %d\n", k_nested_rt_outer(vn9), k_nested_rt_inner(vn9), k_triangular());
    printf("fp=%.3f %.3f\n", k_f64_rt(vn9), k_f32_c6());
    printf("sum_rt=%d %d %d %d %d %d %d %d %d\n", k_sum_rt(vn0), k_sum_rt(vn1), k_sum_rt(vn3), k_sum_rt(vn4), k_sum_rt(vn5), k_sum_rt(vn7), k_sum_rt(vn8), k_sum_rt(vn9), k_sum_rt(vn17));
    printf("sum_le_rt=%d %d %d\n", k_sum_le_rt(vn0), k_sum_le_rt(vn7), k_sum_le_rt(vn8));
    printf("down_rt=%d %d %d\n", k_down_rt(vn9), k_down_rt(vn8), k_down_rt(vn1));
    printf("stride3_rt=%d %d\n", k_stride3_rt(vn17), k_stride3_rt(vn9));
    printf("u_rt=%u l_rt=%ld\n", k_u_rt((unsigned)vn17), k_l_rt(vln));
    printf("multi_carried=%d %d\n", k_multi_carried_rt(vn9), k_multi_carried_rt(vn4));
    printf("mem_dep=%d %d\n", k_mem_dep_rt(vn9), k_mem_dep4());
    printf("break=%d\n", k_break_rt(vn17));
    printf("volatile4=%d\n", k_volatile4());
    printf("flag_toggle=%d %d\n", k_flag_toggle_rt(vn9, vn1), k_flag_toggle_rt(vn9, vn0));
    printf("widen=%ld\n", k_widen_rt(vn9));
    printf("iv_after8=%d\n", k_iv_after8());
    printf("u_neg_init=%d\n", k_u_neg_init());
    printf("one_trip=%d big_step=%d\n", k_one_trip(), k_big_step());
    printf("mixed_back=%d %d %d\n", k_mixed_back(vn3,vn9), k_mixed_back(vn1,vn4), k_mixed_back(vn0,vn1));
    printf("trip16=%d %d\n", k_trip16(), k_trip16_carried());
    printf("desc_carried=%d\n", k_desc_carried());
    printf("live_mixed=%d %d %d\n", k_live_mixed(vn9), k_live_mixed(vn0), k_live_mixed(vn1));
    printf("wide_inv=%ld\n", k_wide_inv(vln));
    return 0;
}

/* 25. One loop mixing every back-value kind: const, invariant param, body
   accumulator, and a cross-phi swap. */
NOINLINE int k_mixed_back(int p, int n) {
    int acc = 0, first = 1, cur = 5, x = 1, y = 2;
    for (int i = 0; i < n; i++) {
        acc += cur * A[i];
        cur = p;
        first = 0;
        int t = x + y; x = y; y = t ^ (i & 3);
    }
    return acc * 7 + first * 3 + cur * 11 + x * 13 + y * 17;
}
/* 26. Max unroll factor boundary (trip = 16). */
NOINLINE int k_trip16(void) {
    int s = 0; for (int i = 0; i < 16; i++) s += A[i] ^ (i * 3); return s;
}
NOINLINE int k_trip16_carried(void) {
    int s = 0, prev = A[0];
    for (int i = 0; i < 16; i++) { s += prev; prev = A[i]; }
    return s * 9 + prev;
}
/* 27. Descending stride 2 with `>=` and IV used in address + carry. */
NOINLINE int k_desc_carried(void) {
    int s = 0, last = -1;
    for (int i = 20; i >= 4; i -= 2) { s += A[i - 4]; last = A[i]; }
    return s * 100 + last;
}
/* 28. Runtime n, accumulator AND a "last" value live-out. */
NOINLINE int k_live_mixed(int n) {
    int s = 0, last = -1;
    for (int i = 0; i < n; i++) { s += A[i] * i; last = A[i + 1]; }
    return s * 100 + last;
}
/* 29. Wide index (i64 iv) with i32 body and a loop-invariant-carried phi. */
NOINLINE long k_wide_inv(int n) {
    long s = 0; int mul = 3;
    for (long i = 0; i < n; i++) { s += LA[i] * mul; mul = n & 1; }
    return s;
}
