/* CPU-model tuning probe kernels (session: cpu_model).
 * Each function isolates one target-dependent selection decision that the
 * x86 tune model must drive.  Compare with scripts/godbolt.py compare. */
#include <stdint.h>
#include <stddef.h>

/* popcnt with dest != src: SNB..CLX carry a false output dependency
 * (uops.info POPCNT_R64_R64 lat 1->1 = 3); ICL+/Zen do not. */
uint64_t popcnt_sum(const uint64_t *p, size_t n) {
    uint64_t s = 0;
    for (size_t i = 0; i < n; i++) s += (uint64_t)__builtin_popcountll(p[i]);
    return s;
}
int ctz_of(uint64_t x) { return __builtin_ctzll(x); }
int clz_of(uint64_t x) { return __builtin_clzll(x); }

/* Variable shifts: SHL r,cl is 3 uops on SNB..CLX, 2 on ICL/ADL-P; SHLX is 1
 * uop everywhere and takes the count in any register. */
uint64_t shl_var(uint64_t x, unsigned k) { return x << k; }
uint64_t shr_var(uint64_t x, unsigned k) { return x >> k; }
int64_t  sar_var(int64_t x, unsigned k)  { return x >> k; }
uint32_t rot_hash(const uint32_t *p, size_t n, unsigned k) {
    uint32_t h = 0;
    for (size_t i = 0; i < n; i++) h = (h << k) ^ p[i];
    return h;
}

/* Block copies: the tune row decides between a vector loop (Generic, no
 * ERMS), `rep movsb` at/above glibc's __x86_rep_movsb_threshold (2112 on
 * FSRM rows, 8192 on the older ERMS rows) and the vector width (16 B at the
 * baseline -march, 32 B with AVX2 unless the part splits 256-bit loads). */
struct blk200 { unsigned char b[200]; };
struct blk2112 { unsigned char b[2112]; };
struct blk4096 { unsigned char b[4096]; };
struct blk8192 { unsigned char b[8192]; };
void copy_200(struct blk200 *d, const struct blk200 *s) { *d = *s; }
void copy_2112(struct blk2112 *d, const struct blk2112 *s) { *d = *s; }
void copy_4096(struct blk4096 *d, const struct blk4096 *s) { *d = *s; }
void copy_8192(struct blk8192 *d, const struct blk8192 *s) { *d = *s; }
