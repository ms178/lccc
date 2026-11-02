// Deterministic sorted-array binary-search kernel.
//
// Self-contained branch/array-access benchmark modeled on common parser/table
// lookup shapes.  No external code is copied; the oracle is fixed by generating
// a simple monotonic table and summing every search result.
#include <stdint.h>
#include <stddef.h>

enum { N = 4096 };
static int table[N];

static void init(void) {
    for (int i = 0; i < N; ++i)
        table[i] = 3 * i + 7;
}

static int find(int key) {
    int lo = 0, hi = N - 1;
    while (lo <= hi) {
        int mid = lo + ((hi - lo) >> 1);
        int v = table[mid];
        if (v == key)
            return mid;
        if (v < key)
            lo = mid + 1;
        else
            hi = mid - 1;
    }
    return -1;
}

int main(void) {
    init();
    unsigned sum = 0;
    for (int key = -1000; key < 3 * N + 1000; key += 7) {
        int idx = find(key);
        if (idx >= 0) {
            if (table[idx] != key)
                return 1;
            sum += (unsigned)idx;
        }
    }
    return sum == 1198665u ? 0 : 2;
}
