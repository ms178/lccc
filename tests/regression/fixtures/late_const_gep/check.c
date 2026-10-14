#include <stdio.h>
/* Separate oracle TU: keep the actual stack/table observations out of LCCC's
   optimizer, and check every element, not just the originally wrong slot. */
int check(const unsigned short *count, const unsigned char *guard,
          const unsigned short *tree) {
    int bad = 0;
    for (int n = 0; n < 16; n++)
        bad |= count[n] != (n == 8 ? 144 : 0);
    for (int n = 0; n < 256; n++)
        bad |= guard[n] != (unsigned char)n;
    for (int n = 0; n < 288; n++)
        bad |= tree[n] != (n < 144 ? 8 : 0);
    printf("%u %u %u\n", (unsigned)count[8], (unsigned)guard[0], (unsigned)guard[1]);
    return bad;
}
