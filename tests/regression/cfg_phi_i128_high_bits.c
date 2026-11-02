/* Phi constants must not compare equal merely because their low halves match.
 * Defined unsigned arithmetic; checks both paths and observes both halves.
 */
#include <stdio.h>

__attribute__((noinline)) static unsigned __int128 choose(unsigned x) {
    unsigned __int128 v;
    if (x & 1u)
        v = (unsigned __int128)1 << 64;
    else
        v = 0;
    return v;
}

__attribute__((noinline)) static int truth(unsigned x) {
    unsigned __int128 v;
    if (x & 1u)
        v = (unsigned __int128)1 << 64;
    else
        v = 0;
    return v != 0;
}

int main(void) {
    for (unsigned i = 0; i < 8; ++i) {
        unsigned __int128 v = choose(i);
        if ((unsigned long long)v != 0 ||
            (unsigned long long)(v >> 64) != (i & 1u) ||
            truth(i) != (int)(i & 1u))
            return 1;
    }
    puts("wide phi: ok");
    return 0;
}
