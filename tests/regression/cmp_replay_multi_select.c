/* Multi-select compare replay: one compare feeding two selects (min/max
   updates) plus a branch re-emits at every consumer instead of
   materializing setcc+movzbl+test per use (binary_search's lo/hi shape,
   linux_find_bit's select+branch shape). */
#include <stdio.h>

static int data[256];

int main(void) {
    for (int i = 0; i < 256; ++i)
        data[i] = (i * 37 + 11) & 255;
    int lo = 1000000, hi = -1000000;
    long sum = 0;
    for (int i = 0; i < 256; ++i) {
        int v = data[i];
        int c = v < 128;
        int a = v * 3 + 1;
        int b = v * 5 + 2;
        lo = c ? (a < lo ? a : lo) : lo;
        hi = c ? hi : (b > hi ? b : hi);
        if (c)
            sum += a;
        else
            sum -= b;
    }
    printf("%d %d %ld\n", lo, hi, sum);
    int elo = 1000000, ehi = -1000000;
    long esum = 0;
    for (int i = 0; i < 256; ++i) {
        int v = (i * 37 + 11) & 255;
        int a = v * 3 + 1, b = v * 5 + 2;
        if (v < 128) {
            if (a < elo)
                elo = a;
            esum += a;
        } else {
            if (b > ehi)
                ehi = b;
            esum -= b;
        }
    }
    if (lo != elo || hi != ehi || sum != esum) {
        printf("MISMATCH %d %d %ld\n", elo, ehi, esum);
        return 1;
    }
    return 0;
}
