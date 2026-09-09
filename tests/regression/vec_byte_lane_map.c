/*
 * Byte-lane conditional map vectorization (OP-05d) — exhaustive gate.
 *
 * Each kernel is a `unsigned char[]`/`signed char[]` map that the demotion
 * analysis must either lower to 8-bit lanes EXACTLY or refuse outright.  The
 * reference is computed through `volatile` scalars so it can never be
 * vectorized into the same (possibly wrong) form, and every trip count from
 * 0 up is exercised so the packed body, the scalar remainder and the empty
 * loop are all covered.
 *
 * The input is byte-exhaustive: the first 256 elements are 0..255 in order,
 * so EVERY possible input byte hits every kernel at every alignment.
 */
#include <stdio.h>
#include <string.h>
#include <stdlib.h>

#define N 1031  /* prime: forces an odd remainder for both VF=16 and VF=32 */

static unsigned char usrc[N], udst[N], uref[N];
static signed char   ssrc[N], sdst[N], sref[N];

/* ---- kernels ---------------------------------------------------------- */
void k_fold_lower(unsigned char *restrict d, const unsigned char *restrict s, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) { unsigned char c = s[i]; if (c >= 'A' && c <= 'Z') c += 32; d[i] = c; }
}
void k_fold_upper(unsigned char *restrict d, const unsigned char *restrict s, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) { unsigned char c = s[i]; if (c >= 'a' && c <= 'z') c -= 32; d[i] = c; }
}
void k_classify(unsigned char *restrict d, const unsigned char *restrict s, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) { unsigned char c = s[i];
        d[i] = (unsigned char)(((c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z')) ? 1 : 0); }
}
void k_clamp(unsigned char *restrict d, const unsigned char *restrict s, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) { unsigned char c = s[i];
        if (c < 16) c = 16; if (c > 240) c = 240; d[i] = c; }
}
void k_wrap_add(unsigned char *restrict d, const unsigned char *restrict s, unsigned long n) {
    /* deliberate 8-bit wraparound: proves the ring homomorphism, not just
       the in-range case */
    for (unsigned long i = 0; i < n; ++i) d[i] = (unsigned char)(s[i] + 200u);
}
void k_mask_xor(unsigned char *restrict d, const unsigned char *restrict s, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) d[i] = (unsigned char)((s[i] ^ 0x5Au) & 0xF3u);
}
void k_eq_replace(unsigned char *restrict d, const unsigned char *restrict s, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) { unsigned char c = s[i]; d[i] = (c == '\t') ? ' ' : c; }
}
void k_wrap_then_cmp(unsigned char *restrict d, const unsigned char *restrict s, unsigned long n) {
    /* The compare operand is a WRAPPED byte sum. In 8-bit lanes the sum
       wraps; in 32-bit it does not. The analysis must either model the
       truncation or refuse — never compare the unwrapped value. */
    for (unsigned long i = 0; i < n; ++i) {
        unsigned char t = (unsigned char)(s[i] + 100u);
        d[i] = (t < 50u) ? 0xFFu : t;
    }
}
void k_signed_clamp(signed char *restrict d, const signed char *restrict s, unsigned long n) {
    /* signed char: sign-extended leaves, negative constants */
    for (unsigned long i = 0; i < n; ++i) { signed char c = s[i];
        if (c < -40) c = -40; if (c > 40) c = 40; d[i] = c; }
}
void k_signed_neg(signed char *restrict d, const signed char *restrict s, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) { signed char c = s[i]; d[i] = (signed char)(c < 0 ? -c : c); }
}
void k_shift_refuse(unsigned char *restrict d, const unsigned char *restrict s, unsigned long n) {
    /* x86 has no packed byte shift: the analysis MUST refuse and stay
       scalar. Correctness is the only thing asserted here. */
    for (unsigned long i = 0; i < n; ++i) d[i] = (unsigned char)((s[i] >> 1) | (s[i] << 7));
}
void k_mul_refuse(unsigned char *restrict d, const unsigned char *restrict s, unsigned long n) {
    /* no packed byte multiply either */
    for (unsigned long i = 0; i < n; ++i) d[i] = (unsigned char)(s[i] * 3u);
}

/* ---- references (volatile => never vectorized) ------------------------ */
#define UREF(EXPR) do { for (int i = 0; i < N; ++i) { volatile unsigned char vc = usrc[i]; unsigned char c = vc; uref[i] = (unsigned char)(EXPR); } } while (0)
#define SREF(EXPR) do { for (int i = 0; i < N; ++i) { volatile signed char vc = ssrc[i]; signed char c = vc; sref[i] = (signed char)(EXPR); } } while (0)

static int fails = 0;

static void runu(const char *name, void (*k)(unsigned char*, const unsigned char*, unsigned long), void (*r)(void)) {
    for (unsigned long n = 0; n <= N; n = (n < 70 ? n + 1 : n * 3 + 1)) {
        unsigned long m = n > N ? N : n;
        memset(udst, 0xAB, sizeof udst);
        r();
        k(udst, usrc, m);
        for (unsigned long i = 0; i < m; ++i)
            if (udst[i] != uref[i]) {
                printf("FAIL %s n=%lu i=%lu src=%02x got=%02x want=%02x\n", name, m, i, usrc[i], udst[i], uref[i]);
                if (++fails > 8) exit(1);
                break;
            }
    }
}
static void runs(const char *name, void (*k)(signed char*, const signed char*, unsigned long), void (*r)(void)) {
    for (unsigned long n = 0; n <= N; n = (n < 70 ? n + 1 : n * 3 + 1)) {
        unsigned long m = n > N ? N : n;
        memset(sdst, 0xAB, sizeof sdst);
        r();
        k(sdst, ssrc, m);
        for (unsigned long i = 0; i < m; ++i)
            if (sdst[i] != sref[i]) {
                printf("FAIL %s n=%lu i=%lu src=%d got=%d want=%d\n", name, m, i, ssrc[i], sdst[i], sref[i]);
                if (++fails > 8) exit(1);
                break;
            }
    }
}

static void r_fold_lower(void){ UREF((c >= 'A' && c <= 'Z') ? c + 32 : c); }
static void r_fold_upper(void){ UREF((c >= 'a' && c <= 'z') ? c - 32 : c); }
static void r_classify(void){ UREF(((c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z')) ? 1 : 0); }
static void r_clamp(void){ UREF(c < 16 ? 16 : (c > 240 ? 240 : c)); }
static void r_wrap_add(void){ UREF(c + 200u); }
static void r_mask_xor(void){ UREF((c ^ 0x5Au) & 0xF3u); }
static void r_eq_replace(void){ UREF(c == '\t' ? ' ' : c); }
static void r_wrap_then_cmp(void){ UREF((unsigned char)(c + 100u) < 50u ? 0xFFu : (unsigned char)(c + 100u)); }
static void r_signed_clamp(void){ SREF(c < -40 ? -40 : (c > 40 ? 40 : c)); }
static void r_signed_neg(void){ SREF(c < 0 ? -c : c); }
static void r_shift_refuse(void){ UREF((c >> 1) | (c << 7)); }
static void r_mul_refuse(void){ UREF(c * 3u); }

int main(void) {
    for (int i = 0; i < N; ++i) {
        usrc[i] = (unsigned char)(i < 256 ? i : (i * 167 + 13));
        ssrc[i] = (signed char)usrc[i];
    }
    runu("fold_lower",    k_fold_lower,    r_fold_lower);
    runu("fold_upper",    k_fold_upper,    r_fold_upper);
    runu("classify",      k_classify,      r_classify);
    runu("clamp",         k_clamp,         r_clamp);
    runu("wrap_add",      k_wrap_add,      r_wrap_add);
    runu("mask_xor",      k_mask_xor,      r_mask_xor);
    runu("eq_replace",    k_eq_replace,    r_eq_replace);
    runu("wrap_then_cmp", k_wrap_then_cmp, r_wrap_then_cmp);
    runs("signed_clamp",  k_signed_clamp,  r_signed_clamp);
    runs("signed_neg",    k_signed_neg,    r_signed_neg);
    runu("shift_refuse",  k_shift_refuse,  r_shift_refuse);
    runu("mul_refuse",    k_mul_refuse,    r_mul_refuse);
    if (fails) { puts("VALIDATION FAILED"); return 1; }
    puts("VALIDATION OK");
    return 0;
}
