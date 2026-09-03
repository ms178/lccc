/* Regression: typed i128 16-byte stores and loads must be bit-exact for
 * every shape the MachInst Mov128 route owns — const, zero, slot-homed
 * value through a pointer, packed-member (unaligned) destination, array
 * round-trips, and the volatile shapes that must keep the mature path's
 * two 8-byte halves.  The suite diffs the runtime output against GCC;
 * the shapes below additionally self-check so a standalone run fails
 * loudly. */
#include <stdio.h>
#include <string.h>

struct __attribute__((packed)) P128 {
    char c;
    unsigned __int128 v;
};

static unsigned failures;

static void check(const char *what, unsigned long long lo, unsigned long long hi,
                  unsigned long long elo, unsigned long long ehi) {
    if (lo != elo || hi != ehi) {
        printf("FAIL %s: got %016llx%016llx want %016llx%016llx\n", what, hi, lo, ehi, elo);
        failures++;
    } else {
        printf("ok %s %016llx%016llx\n", what, hi, lo);
    }
}

static unsigned __int128 g_sink;

int main(void) {
    const unsigned __int128 a =
        ((unsigned __int128)0xDEADBEEFCAFEBABEULL << 64) | 0x1234567890ABCDEFULL;
    const unsigned long long alo = 0x1234567890ABCDEFULL;
    const unsigned long long ahi = 0xDEADBEEFCAFEBABEULL;

    /* const store (immediate halves) */
    unsigned __int128 c = ((unsigned __int128)3 << 100) | 3;
    check("const", (unsigned long long)c, (unsigned long long)(c >> 64), 3, 1 << 4);

    /* zero store */
    unsigned __int128 z = 0;
    check("zero", (unsigned long long)z, (unsigned long long)(z >> 64), 0, 0);

    /* value store through a pointer (slot-homed source) */
    unsigned __int128 b = 0;
    unsigned __int128 *p = &b;
    *p = a;
    check("value-store", (unsigned long long)b, (unsigned long long)(b >> 64), alo, ahi);

    /* array round-trip: stores and loads of distinct slots */
    unsigned __int128 arr[4];
    for (int i = 0; i < 4; i++) {
        arr[i] = a + ((unsigned __int128)i << 64);
    }
    unsigned __int128 sum = 0;
    for (int i = 0; i < 4; i++) {
        sum += arr[i];
    }
    check("array-roundtrip", (unsigned long long)sum, (unsigned long long)(sum >> 64),
          0x1234567890ABCDEFULL * 4, 0xDEADBEEFCAFEBABEULL * 4 + 6);

    /* packed (unaligned) destination: must stay byte-exact */
    struct P128 pk;
    memset(&pk, 0, sizeof(pk));
    pk.v = a;
    check("packed", (unsigned long long)pk.v, (unsigned long long)(pk.v >> 64), alo, ahi);

    /* struct-copy shaped member stores */
    struct P128 pk2;
    memset(&pk2, 0, sizeof(pk2));
    pk2.c = 7;
    pk2.v = pk.v;
    check("member-copy", (unsigned long long)pk2.v, (unsigned long long)(pk2.v >> 64), alo, ahi);

    /* volatile: observable via the mature two-half path */
    volatile unsigned __int128 vv;
    vv = a;
    check("volatile", (unsigned long long)vv, (unsigned long long)(vv >> 64), alo, ahi);

    g_sink = a;
    check("global", (unsigned long long)g_sink, (unsigned long long)(g_sink >> 64), alo, ahi);

    if (failures) {
        return 1;
    }
    printf("all i128 pair shapes ok\n");
    return 0;
}
