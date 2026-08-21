/* Microbenchmark: many interchangeable file-scope bases (fannkuch-shaped).
 *
 * The frontend emits one GlobalAddr per access. Without CSE the three
 * arrays pin a dozen live address values; with class-aware CSE the
 * must-materialize uses share one SSA value per symbol. */
#include <stdio.h>
#include <stdlib.h>

static int perm[128], perm1[128], count[128];

__attribute__((noinline))
static int mix(int *a, int *b, int *c, int n) {
    int s = 0;
    for (int i = 0; i < n; i++) {
        s += a[i] + b[i] + c[i];
        a[i] = s;
    }
    return s;
}

__attribute__((noinline))
int kernel(int n) {
    int acc = 0;
    for (int r = 0; r < n; r++) {
        acc += mix(perm, perm1, count, 128);
        acc += mix(perm1, count, perm, 128);
        acc += mix(count, perm, perm1, 128);
    }
    return acc;
}

int main(int argc, char **argv) {
    int n = argc > 1 ? atoi(argv[1]) : 2000;
    for (int i = 0; i < 128; i++) {
        perm[i] = i;
        perm1[i] = 128 - i;
        count[i] = i * i;
    }
    printf("%d\n", kernel(n));
    return 0;
}
