/*
 * vec_dead_remainder_shapes.c — shape corpus for ZERO-REM-1.
 *
 * One tiny function per shape, so the per-function loop census in
 * `check_vec_remainder_shapes.py` can assert EXACTLY how many loops each
 * shape compiles to.  Every body is `d[i] = s[i] + k` with a constant `k`:
 * integer/floating ADD is available at both the SSE2 (x86-64) and AVX2
 * (x86-64-v3) baselines, so the shapes vectorize on both and a loop count of
 * 1 really means "one packed loop, no scalar mirror" rather than "the loop
 * stayed scalar".
 *
 * The contract under test:
 *
 *   exact-multiple trip count  -> ONE loop  (the dead mirror is omitted)
 *   non-multiple trip count    -> TWO loops (packed loop + scalar tail)
 *   may-alias stream           -> TWO loops (the mirror is the dependence
 *                                 guard's fallback and must be kept)
 *
 * Element counts are exact multiples of the packed width under BOTH baselines
 * (`u32`: 32 % 8 == 32 % 4 == 0; `u8`: 64 % 32 == 64 % 16 == 0; ...), so one
 * table of expectations holds for x86-64 and x86-64-v3 alike.
 */
typedef unsigned char u8;
typedef unsigned short u16;
typedef unsigned int u32;
typedef unsigned long long u64;

/* ── exact multiples: mirror is dead ───────────────────────────────────── */

void shape_u32_exact(u32 *__restrict d, const u32 *__restrict s) {
    int i;
    for (i = 0; i < 32; i++) d[i] = s[i] + 5u;
}

void shape_u8_exact(u8 *__restrict d, const u8 *__restrict s) {
    int i;
    for (i = 0; i < 64; i++) d[i] = (u8)(s[i] + 5u);
}

void shape_u16_exact(u16 *__restrict d, const u16 *__restrict s) {
    int i;
    for (i = 0; i < 32; i++) d[i] = (u16)(s[i] + 5u);
}

void shape_u64_exact(u64 *__restrict d, const u64 *__restrict s) {
    int i;
    for (i = 0; i < 8; i++) d[i] = s[i] + 5u;
}

void shape_i32_exact(int *__restrict d, const int *__restrict s) {
    int i;
    for (i = 0; i < 32; i++) d[i] = s[i] + 5;
}

void shape_f32_exact(float *__restrict d, const float *__restrict s) {
    int i;
    for (i = 0; i < 32; i++) d[i] = s[i] + 5.0f;
}

void shape_f64_exact(double *__restrict d, const double *__restrict s) {
    int i;
    for (i = 0; i < 8; i++) d[i] = s[i] + 5.0;
}

/* Two source streams on the shared byte induction variable. */
void shape_two_src_exact(u32 *__restrict d, const u32 *__restrict a,
                         const u32 *__restrict b) {
    int i;
    for (i = 0; i < 32; i++) d[i] = a[i] + b[i] + 5u;
}

/* The counter escapes: the trip count is materialised, the mirror is not. */
int shape_escape_exact(u32 *__restrict d, const u32 *__restrict s) {
    int i;
    for (i = 0; i < 32; i++) d[i] = s[i] + 5u;
    return i;
}

/* ── non-multiples: the scalar tail must remain ────────────────────────── */

void shape_u32_tail(u32 *__restrict d, const u32 *__restrict s) {
    int i;
    for (i = 0; i < 33; i++) d[i] = s[i] + 5u;
}

void shape_u8_tail(u8 *__restrict d, const u8 *__restrict s) {
    int i;
    for (i = 0; i < 67; i++) d[i] = (u8)(s[i] + 5u);
}

void shape_u16_tail(u16 *__restrict d, const u16 *__restrict s) {
    int i;
    for (i = 0; i < 35; i++) d[i] = (u16)(s[i] + 5u);
}

void shape_u64_tail(u64 *__restrict d, const u64 *__restrict s) {
    int i;
    for (i = 0; i < 9; i++) d[i] = s[i] + 5u;
}

void shape_f32_tail(float *__restrict d, const float *__restrict s) {
    int i;
    for (i = 0; i < 35; i++) d[i] = s[i] + 5.0f;
}

/* ── may-alias: the mirror is the guard's fallback, keep it ────────────── */

void shape_alias_exact(u32 *d, const u32 *s) {
    int i;
    for (i = 0; i < 32; i++) d[i] = s[i] + 5u;
}

void shape_alias_fwd(u32 *d, const u32 *s) {
    int i;
    for (i = 0; i < 32; i++) d[i] = s[i + 1] + 5u;
}
