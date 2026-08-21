// Deterministic ASCII case-folding / byte-class kernel.
//
// This is not a copy of any project source; it exercises the same hot shapes
// found across gzip/expat/curl parsers: byte loads, dependent compares,
// branchless selects, pointer loops, and a checksum-like accumulator.  The
// alphabet/data are generated deterministically so LCCC/GCC/Clang/ICX have an
// exact cross-compiler oracle.
#include <stdint.h>
#include <stddef.h>

enum { N = 1u << 16 };
static unsigned char data[N];
static unsigned char folded[N];

static void init(void) {
    for (unsigned i = 0; i < N; ++i) {
        unsigned v = (i * 1103515245u + 12345u) >> 16;
        data[i] = (unsigned char)((v % 95u) + 32u);
    }
}

static unsigned fold_all(void) {
    unsigned sum = 0;
    for (size_t i = 0; i < N; ++i) {
        unsigned c = data[i];
        if (c >= 'A' && c <= 'Z')
            c += (unsigned)('a' - 'A');
        folded[i] = (unsigned char)c;
        sum = (sum << 5) + sum + c;
    }
    return sum;
}

int main(void) {
    init();
    unsigned s = fold_all();
    if (folded[0] != ' ' || folded[1] != '7' || folded[2] != 'n')
        return 1;
    return (s == 2571300113u) ? 0 : 2;
}
