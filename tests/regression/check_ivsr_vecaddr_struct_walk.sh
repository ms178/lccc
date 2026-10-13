#!/usr/bin/env bash
# IVSR struct-walk / vector-memory address-use gate (nbody pair-loop family).
#
# WHAT THIS PINS.  The pointer-recurrence machinery in
# `iv_strength_reduce` used to see ONLY `GetElementPtr` offset uses of a
# derived `iv * stride` value.  Two real shapes escaped it:
#   1. GVN Copy glue: the surviving multiply is reached through
#      `Copy(mul)`, so the literal offset operand never matched (this is
#      what every post-GVN frontend IR looks like).
#   2. Vector-memory `(base, index, disp)` triples: nbody's pair loop
#      feeds `j * 56` to `VecLoad*`/`VecStore*` arg triples ONLY, so the
#      multiply survived at 2 imulq $56 per outer iteration and every
#      struct field re-materialised its base (`leaq bodies(%rip)`,
#      `movslq`, `imulq`, `addq` — 5 instructions per outer iteration)
#      while GCC kept one marching pointer (advance: 1.72x GCC).
#
# The fix collects GEP offsets through alias glue, collects vec triples as
# the SAME address use, groups them onto one pointer recurrence per
# (IV, stride, invariant base), rewrites triples in place to
# `(ptr, Const(0), disp)` (arity preserved — `vec_intrinsic_access` and
# the emitter index slots positionally), and chains affine siblings
# (`base + (i+1)*56` becomes `GEP(ptr_i, 56)`, no second recurrence).
#
# THE SHAPE GATE (the point of the whole exercise): the compiled pair-walk
# loop must contain ZERO stride multiplies and exactly ONE hoisted
# `bodies` materialisation.  If a future change (RA, another pass, the
# recurrence logic) reintroduces per-iteration address arithmetic, this
# gate fails loudly instead of silently handing the win back.
#
# THE CORRECTNESS GATE: a self-checking pair-walk program (integer math,
# bit-exact reference vs optimized loop in the same TU) must pass built
# with lccc at -O0..-O3, in 64- and 32-bit mode, and under the
# CCC_NO_IVSR_PTR_ADD=1 kill switch (an escape hatch nobody exercises is
# how a switch rots into a no-op or a crash).
set -u
CCC=${CCC:-target/fastbuild/lccc}
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
fails=0
note() { printf '%s\n' "$*"; }
bad()  { note "FAIL: $*"; fails=$((fails+1)); }

if [ ! -x "$CCC" ]; then
    note "FAIL: compiler $CCC not found; build it first (scripts/build_lccc_fast.sh)"
    exit 1
fi

# ── host 32-bit probe (same doctrine as check_ivsr_domains.sh) ─────────
inc=$("${GCC_BIN:-gcc}" -print-file-name=include)
if ! printf 'int main(void) { return 0; }\n' >"$TMP/i386-probe.c" \
   || ! ${GCC_BIN:-gcc} -m32 "$TMP/i386-probe.c" -o "$TMP/i386-probe" 2>"$TMP/i386-probe.err" \
   || ! "$TMP/i386-probe" >/dev/null 2>&1; then
    bad "host, not lccc: gcc -m32 cannot build and run a trivial program; install gcc-multilib and libc6-dev-i386"
    sed -n '1,2p' "$TMP/i386-probe.err" >&2
fi

# ── the self-checking pair-walk (integer math: bit-exact reference) ────
cat >"$TMP/pairwalk.c" <<'EOF'
#include <stdio.h>
#include <stdlib.h>

typedef struct {
    long long x, y, z, vx, vy, vz, mass;
} Body;

static Body bodies[6];

/* The pair-walk shape under test: nested index loops over a struct array
   with a j = i + 1 start and a six-field interaction.  The addressing is
   exactly the nbody pattern the recurrence must cover. */
static void advance(long long dt, int n) {
    for (int step = 0; step < n; step++) {
        for (int i = 0; i < 6; i++) {
            for (int j = i + 1; j < 6; j++) {
                long long dx = bodies[i].x - bodies[j].x;
                long long dy = bodies[i].y - bodies[j].y;
                long long dz = bodies[i].z - bodies[j].z;
                long long d2 = dx * dx + dy * dy + dz * dz + 1;
                long long f = dt * bodies[j].mass / d2;
                bodies[i].vx += dx * f;
                bodies[i].vy += dy * f;
                bodies[i].vz += dz * f;
                bodies[j].vx -= dx * f;
                bodies[j].vy -= dy * f;
                bodies[j].vz -= dz * f;
            }
        }
        for (int i = 0; i < 6; i++) {
            bodies[i].x += dt * bodies[i].vx;
            bodies[i].y += dt * bodies[i].vy;
            bodies[i].z += dt * bodies[i].vz;
        }
    }
}

int main(int argc, char **argv) {
    int n = argc > 1 ? atoi(argv[1]) : 2000;
    long long dt = 2;
    Body ref[6];
    for (int i = 0; i < 6; i++) {
        bodies[i].x = 3 * i + 1;
        bodies[i].y = 5 * i - 2;
        bodies[i].z = 7 * i + 4;
        bodies[i].vx = 0;
        bodies[i].vy = 0;
        bodies[i].vz = 0;
        bodies[i].mass = 11 + i;
        ref[i] = bodies[i];
    }
    /* Reference: the same interaction order, kept in plain local scalars
       with no struct-array walking, so no compiler can strength-reduce the
       two legs together.  Same integer operations, same order -> the two
       legs are bit-exact by construction. */
    for (int step = 0; step < n; step++) {
        for (int i = 0; i < 6; i++) {
            for (int j = i + 1; j < 6; j++) {
                long long dx = ref[i].x - ref[j].x;
                long long dy = ref[i].y - ref[j].y;
                long long dz = ref[i].z - ref[j].z;
                long long d2 = dx * dx + dy * dy + dz * dz + 1;
                long long f = dt * ref[j].mass / d2;
                ref[i].vx += dx * f;
                ref[i].vy += dy * f;
                ref[i].vz += dz * f;
                ref[j].vx -= dx * f;
                ref[j].vy -= dy * f;
                ref[j].vz -= dz * f;
            }
        }
        for (int i = 0; i < 6; i++) {
            ref[i].x += dt * ref[i].vx;
            ref[i].y += dt * ref[i].vy;
            ref[i].z += dt * ref[i].vz;
        }
    }
    advance(dt, n);
    int bad = 0;
    for (int i = 0; i < 6; i++) {
        if (bodies[i].x != ref[i].x || bodies[i].y != ref[i].y || bodies[i].z != ref[i].z
            || bodies[i].vx != ref[i].vx || bodies[i].vy != ref[i].vy
            || bodies[i].vz != ref[i].vz) {
            bad = 1;
            fprintf(stderr, "mismatch at body %d\n", i);
        }
    }
    if (bad) {
        fprintf(stderr, "FAIL: pairwalk optimized loop diverged from reference\n");
        return 1;
    }
    long long sum = 0;
    for (int i = 0; i < 6; i++)
        sum += bodies[i].x + bodies[i].y + bodies[i].z + bodies[i].vx + bodies[i].vy + bodies[i].vz;
    printf("ok %lld\n", sum);
    return 0;
}
EOF

# ── the shape program (exactly the pair-walk advance, nothing else) ────
cat >"$TMP/pairwalk_shape.c" <<'EOF'
typedef struct {
    double x, y, z, vx, vy, vz, mass;
} Body;

Body bodies[5];

void advance(double dt, int n) {
    for (int step = 0; step < n; step++) {
        for (int i = 0; i < 5; i++) {
            for (int j = i + 1; j < 5; j++) {
                double dx = bodies[i].x - bodies[j].x;
                double dy = bodies[i].y - bodies[j].y;
                double dz = bodies[i].z - bodies[j].z;
                double dist2 = dx * dx + dy * dy + dz * dz + 1e-9;
                double mag = dt / (dist2 * __builtin_sqrt(dist2));
                bodies[i].vx -= dx * bodies[j].mass * mag;
                bodies[i].vy -= dy * bodies[j].mass * mag;
                bodies[i].vz -= dz * bodies[j].mass * mag;
                bodies[j].vx += dx * bodies[i].mass * mag;
                bodies[j].vy += dy * bodies[i].mass * mag;
                bodies[j].vz += dz * bodies[i].mass * mag;
            }
        }
    }
}
EOF

# ── correctness: every opt level, 64-bit, and the kill switch ──────────
for opt in -O0 -O1 -O2 -O3; do
    if "$CCC" "$opt" "$TMP/pairwalk.c" -o "$TMP/pw64" 2>"$TMP/cc.err"; then
        "$TMP/pw64" 2000 >/dev/null || bad "pairwalk $opt 64-bit self-check failed"
    else
        bad "pairwalk $opt 64-bit did not compile"
        sed -n '1,4p' "$TMP/cc.err" >&2
    fi
    if "$CCC" "$opt" -m32 -msse2 -I"$inc" "$TMP/pairwalk.c" -o "$TMP/pw32" 2>"$TMP/cc32.err"; then
        "$TMP/pw32" 2000 >/dev/null || bad "pairwalk $opt i386 self-check failed"
    else
        bad "pairwalk $opt i386 did not compile"
        sed -n '1,4p' "$TMP/cc32.err" >&2
    fi
    if CCC_NO_IVSR_PTR_ADD=1 "$CCC" "$opt" "$TMP/pairwalk.c" -o "$TMP/pwoff" 2>"$TMP/ccoff.err"; then
        "$TMP/pwoff" 2000 >/dev/null || bad "pairwalk $opt CCC_NO_IVSR_PTR_ADD=1 self-check failed"
    else
        bad "pairwalk $opt CCC_NO_IVSR_PTR_ADD=1 did not compile"
        sed -n '1,4p' "$TMP/ccoff.err" >&2
    fi
done

# ── the shape gate (x86-64 only: the ptr-add arm is off on ILP32) ──────
if "$CCC" -O2 -S "$TMP/pairwalk_shape.c" -o "$TMP/shape.s" 2>"$TMP/shape.err"; then
    imuls=$(grep -cF 'imulq $56' "$TMP/shape.s" || true)
    bases=$(grep -c 'leaq[[:space:]]*bodies' "$TMP/shape.s" || true)
    if [ "$imuls" -ne 0 ]; then
        bad "pair-walk reintroduced stride multiplies: imulq \$56 count = $imuls (want 0)"
        grep -nF 'imulq $56' "$TMP/shape.s" | head -5 >&2
    fi
    if [ "$bases" -ne 1 ]; then
        bad "pair-walk materialises the struct base $bases times (want exactly 1 hoisted leaq bodies)"
        grep -n 'leaq[[:space:]]*bodies' "$TMP/shape.s" | head -5 >&2
    fi
    # The kill switch is allowed to change the shape (that is its purpose),
    # but it must still produce runnable code.
    if ! CCC_NO_IVSR_PTR_ADD=1 "$CCC" -O2 -S "$TMP/pairwalk_shape.c" -o "$TMP/shape_off.s" 2>"$TMP/shape_off.err"; then
        bad "pair-walk shape build under CCC_NO_IVSR_PTR_ADD=1 did not compile"
    fi
else
    bad "pair-walk shape program did not compile"
    sed -n '1,4p' "$TMP/shape.err" >&2
fi

if [ "$fails" -ne 0 ]; then
    note "check_ivsr_vecaddr_struct_walk: $fails failure(s)"
    exit 1
fi
note "check_ivsr_vecaddr_struct_walk: OK"
