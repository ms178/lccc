/* Isolated ISel/peephole ROI kernels for Godbolt oracles.
 * Each function is noinline so CE keeps a named body.
 *
 *   scripts/codegen_oracle.py engineering/kernels/isel_roi.c \
 *     --function popcount32 --function clz32 --function andn32 \
 *     --local target/fastbuild/lccc \
 *     --flags '-O3 -march=x86-64-v3' \
 *     --oracles gcc16.2,clang23.1,icc,icx
 */
#define NI __attribute__((noinline))

unsigned NI popcount32(unsigned x) { return (unsigned)__builtin_popcount(x); }
unsigned long NI popcount64(unsigned long x) { return (unsigned long)__builtin_popcountl(x); }
int NI clz32(unsigned x) { return __builtin_clz(x); }
int NI ctz32(unsigned x) { return __builtin_ctz(x); }
unsigned NI andn32(unsigned a, unsigned b) { return ~a & b; }
unsigned NI blsr32(unsigned x) { return x & (x - 1u); }
unsigned NI blsi32(unsigned x) { return x & -x; }
unsigned NI bzhi32(unsigned x, unsigned n) {
    n &= 31u;
    return n ? (x & ((1u << n) - 1u)) : 0u;
}
unsigned NI bit_test32(unsigned x, unsigned k) { return (x >> (k & 31u)) & 1u; }
unsigned NI min_u32(unsigned a, unsigned b) { return a < b ? a : b; }
int NI max_i32(int a, int b) { return a > b ? a : b; }
int NI abs_i32(int x) { return x < 0 ? -x : x; }
unsigned NI mul3(unsigned x) { return x * 3u; }
unsigned NI mul5(unsigned x) { return x * 5u; }
unsigned NI mul9(unsigned x) { return x * 9u; }
unsigned NI add_imm(unsigned x) { return x + 42u; }
unsigned NI zero(void) { return 0u; }
unsigned NI rotl32(unsigned x, unsigned n) {
    n &= 31u;
    return n ? ((x << n) | (x >> (32u - n))) : x;
}
unsigned NI hash_mul(unsigned x) { return x * 0x9e3779b1u; }
unsigned NI select_inc(unsigned x, unsigned y) { return x ? y + 1u : y; }
int NI cmp0(int x) { return x != 0; }
unsigned NI zext_load(const unsigned char *p) { return *p; }
unsigned NI lea_index(unsigned *a, unsigned i) { return a[i]; }
