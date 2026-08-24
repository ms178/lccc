/*
 * Complete unrolling of nested constant-trip loops (general unroller).
 *
 * Covers three hazards found and fixed during development:
 *  1. cloned inner-loop terminators must rename their compare VALUE uses
 *     (a cloned CondBranch referencing the original compare read a dead
 *     value — infinite loop);
 *  2. the header phi -> Copy replacement must take the NON-LATCH incoming
 *     (taking incoming.first() made the dead phi self-referential with its
 *     back-edge value, leaving iteration 0's IV garbage and silently
 *     skipping its body);
 *  3. the triangular inner init (`j = i+1`) resolves through the const
 *     chain so the fixpoint cascades outer -> inner.
 *
 * Checked elementwise against volatile-anchored references.
 */
#include <stdio.h>
typedef struct { double x, y, m; } B;
static B b[6] = {{1,2,3},{4,5,6},{7,8,9},{10,11,12},{13,14,15},{16,17,18}};

static double e_sum(int n) {
    double e = 0;
    for (int i = 0; i < n; i++) {
        e += 0.5 * b[i].m;
        for (int j = i + 1; j < n; j++)
            e -= b[i].m * b[j].m;
    }
    return e;
}

int main(void) {
    int bad = 0;
    double r = 0;
    for (int i = 0; i < 6; i++) {
        volatile double mi = b[i].m;
        r += 0.5 * mi;
        for (int j = i + 1; j < 6; j++) {
            volatile double mj = b[j].m;
            r -= mi * mj;
        }
    }
    double v = e_sum(6);
    if (v != r) bad++;
    printf("%s\n", bad == 0 ? "OK" : "FAIL");
    return bad;
}
