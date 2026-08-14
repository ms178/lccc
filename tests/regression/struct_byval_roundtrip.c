// Regression: struct-by-value return/passing with mixed scalar fields must
// round-trip exactly. Exercises the aggregate-copy paths (Memcpy forwards,
// SROA-adjacent lowering) with a non-trivial field layout and loop reuse.
#include <stdio.h>

typedef struct { double x, y, z; int id; char tag[8]; } Vec3;

static Vec3 make(int i) {
    Vec3 v;
    v.x = i * 0.25;
    v.y = i * 0.5;
    v.z = i * 0.75;
    v.id = i;
    v.tag[0] = 't';
    v.tag[1] = (char)('0' + (i % 10));
    v.tag[2] = 0;
    return v;
}

static double dist2(Vec3 a, Vec3 b) {
    double dx = a.x - b.x, dy = a.y - b.y, dz = a.z - b.z;
    return dx * dx + dy * dy + dz * dz;
}

typedef struct { Vec3 v[3]; double total; } Group;

int main(void) {
    Group g;
    g.total = 0.0;
    for (int i = 0; i < 3; i++) g.v[i] = make(i * 7 + 1);
    for (int i = 0; i < 3; i++)
        for (int j = i + 1; j < 3; j++)
            g.total += dist2(g.v[i], g.v[j]);
    /* Deterministic: total = sum of squared distances for ids 1,8,15 */
    printf("struct_byval total: %.2f\n", g.total);
    return 0;
}
