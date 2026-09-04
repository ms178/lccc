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
