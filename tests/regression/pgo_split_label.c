// split-label PGO:
//
// PGO v6: critical-edge split blocks must receive MODULE-GLOBAL unique block
// labels. The original implementation allocated split labels from the
// instrumented function's own max label; because the frontend numbers blocks
// from a per-TU counter, the split label of an early function collided with a
// real block label of a later function in the same TU. The emitted assembly
// then contained two `.LBB{id}:` definitions, and branches to the split
// block bound to the WRONG definition — control jumped into the middle of an
// unrelated function (observed in zlib-ng trees.c: pqdownheap's `jl` landed
// in build_tree+191, corrupting the Huffman-tree build and crashing
// minigzip).
//
// f1 (first in the TU) contains a do-while loop with a short-circuit
// condition, which produces a critical instrumented edge and hence a split
// block. f2 (later in the TU) occupies the label range right after f1, so
// f1's split block label collides with f2's real block label under the
// per-function allocation. main recomputes the expected result with an
// independent formulation and exits nonzero on any mismatch.
#include <stdio.h>

__attribute__((noinline)) static int f1(int *a, int n) {
    int i = 0, s = 0;
    do {
        if (a[i] > 0 && a[i] < 500) s += a[i];
        else s -= 1;
        if (a[i] == 123) s += 7;
        i++;
    } while (i < n);
    return s;
}

__attribute__((noinline)) static int f2(int *a, int n) {
    int s = 0;
    for (int i = 0; i < n; i++) {
        if (a[i] & 1) s += a[i];
        else s -= 2;
        if (a[i] > 100) s += 5;
    }
    return s;
}

int main(void) {
    int a[3000];
    for (int i = 0; i < 3000; i++) a[i] = (int)((i * 2654435761u) >> 13);
    int r = f1(a, 3000) + f2(a, 3000) * 3;
    int exp = 0;
    for (int i = 0; i < 3000; i++) {
        int v = a[i];
        int part = 0;
        if (v > 0) {
            if (v < 500) part += v;
            else part -= 1;
        } else {
            part -= 1;
        }
        if (v == 123) part += 7;
        exp += part;
        if (v & 1) exp += 3 * v;
        else exp -= 6;
        if (v > 100) exp += 15;
    }
    if (r != exp) {
        printf("MISMATCH got=%d exp=%d\n", r, exp);
        return 1;
    }
    printf("ok %d\n", r);
    return 0;
}
