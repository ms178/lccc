/* Signed I32 producers feeding pointer-width uses. All arithmetic here is
 * nonoverflowing; both -fwrapv and -fno-wrapv must work. Addresses stay within
 * one real object. Includes negative/positive results and loop-carried sign
 * changes, not just constant scaling. */
#include <stdio.h>
#include <stdint.h>
#define NI __attribute__((noinline))
#define BIN(name, expr) static NI int name(const unsigned char *p, int i, int j) { return p[expr]; }
BIN(add, i + j)
BIN(sub, i - j)
BIN(mul, i * j)
BIN(quot, i / j)
BIN(rem, i % j)
BIN(band, i & j)
BIN(bor, i | j)
BIN(bxor, i ^ j)
BIN(neg, -i)
BIN(inv, ~i)
BIN(param, i)
static NI int pick(const unsigned char *p, int i, int j) {
    int k;
    if (j) k = i; else k = -i;
    return p[k];
}
static NI int up(const unsigned char *p, int i, int j) {
    for (int k = 0; k < j; ++k) i += 2;
    return p[i];
}
static NI int down(const unsigned char *p, int i, int j) {
    for (int k = 0; k < j; ++k) i -= 2;
    return p[i];
}
static NI int load(const unsigned char *p, int i, int j) {
    volatile int k = i;
    (void)j;
    return p[k];
}
static NI int identity(int i) { return i; }
BIN(call, identity(i))
struct row { int (*fn)(const unsigned char *, int, int); int i, j, offset; };
static const struct row rows[] = {
    {add, -2, 1, -1}, {add, 2, -1, 1},
    {sub, 1, 2, -1}, {sub, 2, 1, 1},
    {mul, -1, 3, -3}, {mul, -1, -3, 3},
    {quot, -7, 2, -3}, {quot, -7, -2, 3},
    {rem, -7, 2, -1}, {rem, 7, -2, 1},
    {band, -3, -2, -4}, {band, 3, 2, 2},
    {bor, -4, 1, -3}, {bor, 3, 4, 7},
    {bxor, -4, 1, -3}, {bxor, 3, 1, 2},
    {neg, 1, 0, -1}, {neg, -1, 0, 1},
    {inv, 0, 0, -1}, {inv, -2, 0, 1},
    {param, -1, 0, -1}, {param, 1, 0, 1},
    {pick, 1, 0, -1}, {pick, 1, 1, 1},
    {up, -3, 3, 3}, {up, -9, 3, -3},
    {down, 3, 3, -3}, {down, 9, 3, 3},
    {load, -1, 0, -1}, {load, 1, 0, 1},
    {call, -1, 0, -1}, {call, 1, 0, 1}
};
/* Explicit casts consume low source bits, unlike the raw-wide address uses
 * above. Negative values, unsigned destinations, zero extension, branch joins,
 * volatile loads and a loop must work without redundant def-side extension.
 * Pointer round trips are ELF x86 implementation-defined conversions only;
 * these pointers are never dereferenced. All additions are nonoverflowing. */
static NI int64_t add_long(int x) { return (int64_t)(x + 1); }
static NI uint64_t add_ulong(int x) { return (uint64_t)(x + 1); }
static NI uint64_t add_zext(int x) { return (uint64_t)(uint32_t)(x + 1); }
static NI intptr_t add_pointer(int x) { return (intptr_t)(void *)(intptr_t)(x + 1); }
static NI int64_t pick_long(int x, int flag) {
    int y = flag ? x + 1 : -1;
    return (int64_t)y;
}
static NI int64_t load_long(int x) { volatile int y = x + 1; return (int64_t)y; }
static NI int64_t sum_long(const int *p, int n) {
    int64_t sum = 0;
    for (int k = 0; k < n; ++k) sum += (int64_t)(p[k] + 1);
    return sum;
}
static int explicit_widening(void) {
    static const struct { int input; int64_t signed_result; uint64_t zeroed; } cases[] = {
        {-2147483647 - 1, -2147483647LL, 2147483649ULL},
        {-50000, -49999LL, 4294917297ULL}, {-1, 0, 0}, {0, 1, 1},
        {1, 2, 2}, {50000, 50001, 50001}, {2147483646, 2147483647LL, 2147483647ULL}
    };
    int inputs[7];
    for (unsigned k = 0; k < 7; ++k) {
        volatile int x = cases[k].input;
        int64_t expected = cases[k].signed_result;
        inputs[k] = x;
        if (add_long(x) != expected || add_ulong(x) != (uint64_t)expected ||
            add_zext(x) != cases[k].zeroed || add_pointer(x) != expected ||
            pick_long(x, 1) != expected || pick_long(x, 0) != -1 || load_long(x) != expected) {
            printf("explicit widening mismatch: case %u\n", k);
            return 0;
        }
    }
    return sum_long(inputs, 7) == 5 ? 50 : 0;
}

int main(void) {
    unsigned char data[64];
    for (unsigned k = 0; k < sizeof(data); ++k) data[k] = (unsigned char)(k + 1);
    const unsigned char *p = data + 32;
    unsigned count = sizeof(rows) / sizeof(rows[0]);
    for (unsigned k = 0; k < count; ++k) {
        volatile int i = rows[k].i, j = rows[k].j;
        int actual = rows[k].fn(p, i, j);
        if (actual != p[rows[k].offset]) {
            printf("i32 address mismatch: case %u got %d\n", k, actual);
            return 1;
        }
    }
    int widened = explicit_widening();
    if (widened != 50) return 1;
    count += (unsigned)widened;
    printf("i32 address canonical: PASS %u\n", count);
    return count != 82;
}
