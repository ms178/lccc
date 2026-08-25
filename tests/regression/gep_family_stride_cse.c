/* Regression: struct-array loops whose element stride is not a legal SIB
 * scale (e.g. 56-byte bodies) must not keep one site-local GlobalAddr per
 * field access. Before the OP-34 stride gate, every `s.field` of `bodies[j]`
 * re-materialised `GA + j*56`, IVSR produced seven marching pointers, and
 * the register flood forced a stack-slot relay (nbody: 11 pointers, one
 * load+store per pointer per iteration). The gate lets GVN CSE the
 * `GA + idx*stride` family into one pointer; field offsets fold into
 * displacements. Differential vs GCC. */
#include <stdio.h>

typedef struct { double x, y, z, vx, vy, vz, mass; } Body;

#define N 37
static Body bodies[N];

static void init(void) {
    for (int i = 0; i < N; i++) {
        bodies[i].x = i * 0.5;
        bodies[i].y = i * 1.5;
        bodies[i].z = i * 2.5;
        bodies[i].vx = i * 0.25;
        bodies[i].vy = i * 0.75;
        bodies[i].vz = i * 1.25;
        bodies[i].mass = 1.0 + i;
    }
}

/* Pairwise interaction over the same triangular loop shape as nbody. */
static double advance(double dt) {
    double acc = 0.0;
    for (int i = 0; i < N; i++) {
        for (int j = i + 1; j < N; j++) {
            double dx = bodies[i].x - bodies[j].x;
            double dy = bodies[i].y - bodies[j].y;
            double dz = bodies[i].z - bodies[j].z;
            double d2 = dx * dx + dy * dy + dz * dz;
            double mag = dt / (d2 + 1.0);
            bodies[i].vx -= dx * bodies[j].mass * mag;
            bodies[i].vy -= dy * bodies[j].mass * mag;
            bodies[i].vz -= dz * bodies[j].mass * mag;
            bodies[j].vx += dx * bodies[i].mass * mag;
            bodies[j].vy += dy * bodies[i].mass * mag;
            bodies[j].vz += dz * bodies[i].mass * mag;
            acc += d2;
        }
    }
    return acc;
}

int main(void) {
    init();
    double total = 0.0;
    for (int step = 0; step < 3; step++)
        total += advance(0.01);
    double sum = 0.0;
    for (int i = 0; i < N; i++)
        sum += bodies[i].vx + bodies[i].vy + bodies[i].vz + bodies[i].mass;
    printf("%.12f %.12f\n", total, sum);
    return 0;
}
