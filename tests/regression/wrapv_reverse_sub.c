/* Invariant-minus-IV stays narrow even when the IV/address path is widened.
 * All addresses are masked. INT_MIN - i and (n + 1) deliberately exercise
 * -fwrapv; a widened subtraction followed by a sign extension is NOT valid.
 */
#include <limits.h>
#include <stdio.h>

static unsigned data[64];

__attribute__((noinline))
static unsigned probe(int n, int pivot)
{
    unsigned sum = 0;
    int bound = (n + 1) >> 1;
    for (int i = 0; i < bound; ++i) {
        if (i == 64)
            break;
        int reverse = pivot - i;
        /* Preserve both the sign and the low bits of the narrow subtraction. */
        sum = sum * 33u + data[(unsigned)i & 63u];
        sum ^= data[(unsigned)reverse & 63u];
        sum += (unsigned)(reverse < 0);
        sum ^= (unsigned)((long long)reverse >> 31);
    }
    return sum;
}

int main(void)
{
    static const int values[] = {
        INT_MIN, INT_MIN + 1, -129, -2, -1, 0, 1, 2,
        63, 64, 65, 127, 128, INT_MAX - 1, INT_MAX
    };
    unsigned hash = 2166136261u;
    for (unsigned i = 0; i < 64; ++i)
        data[i] = i * 2654435761u ^ (i << 9);
    for (unsigned i = 0; i < sizeof(values) / sizeof(values[0]); ++i)
        for (unsigned j = 0; j < sizeof(values) / sizeof(values[0]); ++j)
            hash = (hash ^ probe(values[i], values[j])) * 16777619u;
    printf("%08x\n", hash);
    return 0;
}
