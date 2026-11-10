#include <stdio.h>
#include <string.h>
/* The property that actually matters: enabling the F16C hardware path must not
 * change any value.  Dump the float bits for every binary16 pattern class, so
 * two builds (-mf16c vs not) can be diffed byte for byte. */
int main(void) {
    unsigned short patterns[] = {0x0000, 0x8000, 0x0001, 0x03FF, 0x0400, 0x3C00,
                                 0xBC00, 0x7BFF, 0x7C00, 0xFC00, 0x3555, 0x4248,
                                 0x5140, 0x5B9E};
    for (unsigned i = 0; i < sizeof patterns / sizeof *patterns; i++) {
        _Float16 h; memcpy(&h, &patterns[i], 2);
        float f = (float)h;
        unsigned b; memcpy(&b, &f, 4);
        printf("%04x %08x\n", patterns[i], b);
    }
    return 0;
}
