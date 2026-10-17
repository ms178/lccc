#!/usr/bin/env bash
# x86-64 vector-shaped varargs ABI: SysV passes a vector variadic argument in
# ONE SSE register (fp_offset advances by 16 regardless of width), INTEGER
# carriers (<=4-byte and bit-packed bool shapes) in one GP slot, and
# >16-byte / carrier-less shapes in one 16-aligned overflow slot. Both the
# caller (value in XMM + %al census) and the va_arg consumer (register-save
# fetch, never the pointer convention) must honour this or the two sides of
# every *other* toolchain disagree with LCCC.
#
# Oracles:
#   * GCC 16 cross legs — vector_size(8)/(16) shapes, both link directions
#     (lccc caller -> gcc consumer, gcc caller -> lccc consumer) plus the two
#     self legs. GCC implements vector_size natively, so a mismatch here is a
#     real interop defect, and neither side can quietly redefine the ABI.
#   * LCCC-only ext_vector_type legs — GCC ignores ext_vector_type entirely
#     (measured: the attribute is dropped and the typedef degrades to the
#     scalar element type), so the Clang-shaped fixtures run as LCCC
#     round-trips with the expected values pinned.
set -euo pipefail
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
ccc=${CCC:-$root/target/fastbuild/lccc}
gcc_bin=${GCC:-gcc}

[ -x "$ccc" ] || { echo "FAIL: compiler not built: $ccc" >&2; exit 1; }

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }

# ---------------------------------------------------------------- fixtures --
# Cross-oracle consumer/caller: vector_size shapes only (GCC is a semantic
# oracle for those), interleaved with scalar varargs so a wrong fetch width
# or offset advance poisons every later argument, not just its own.
cat > "$tmp/consumer.c" <<'EOF'
#include <stdarg.h>
typedef long long v1 __attribute__((vector_size(8)));
typedef int v4 __attribute__((vector_size(16)));
long long consumer(int n, ...) {
    va_list ap;
    va_start(ap, n);
    v1 a = va_arg(ap, v1);
    long l = va_arg(ap, long);
    v4 b = va_arg(ap, v4);
    double d = va_arg(ap, double);
    va_end(ap);
    return a[0] + l + b[0] + b[3] + (long long)d + n;
}
EOF
cat > "$tmp/driver.c" <<'EOF'
#include <stdio.h>
typedef long long v1 __attribute__((vector_size(8)));
typedef int v4 __attribute__((vector_size(16)));
long long consumer(int, ...);
int main(void) {
    v1 a = {100};
    v4 b = {1, 2, 3, 4};
    long long r = consumer(7, a, 55L, b, 0.5);
    printf("%lld\n", r);
    return r == 167 ? 0 : 1;
}
EOF

compile() { # compile <compiler> <src> <obj>
    "$1" -O1 -c "$2" -o "$3" || fail "$1 cannot compile $2"
}

gcc_ok=1
command -v "$gcc_bin" >/dev/null 2>&1 || gcc_ok=0
if [[ $gcc_ok == 1 ]]; then
    if ! "$gcc_bin" -O1 -c "$tmp/consumer.c" -o "$tmp/consumer_g.o" 2>/dev/null; then
        gcc_ok=0
    fi
fi
if [[ $gcc_ok == 0 ]]; then
    if [[ ${LCCC_REQUIRE_GCC:-0} == 1 ]]; then
        fail "required GCC oracle not found: $gcc_bin"
    fi
    echo "SKIP: GCC cross-ABI oracle not found: $gcc_bin"
fi

"$ccc" -O1 -c "$tmp/consumer.c" -o "$tmp/consumer_l.o" || fail "lccc consumer leg"
"$ccc" -O1 -c "$tmp/driver.c" -o "$tmp/driver_l.o" || fail "lccc driver leg"

if [[ $gcc_ok == 1 ]]; then
    "$gcc_bin" -O1 -c "$tmp/driver.c" -o "$tmp/driver_g.o"
    for caller in lccc gcc; do
        for callee in lccc gcc; do
            bin="$tmp/x_${caller}_${callee}"
            "$gcc_bin" -O1 -o "$bin" "$tmp/driver_${caller:0:1}.o" \
                "$tmp/consumer_${callee:0:1}.o"
            out=$("$bin") || fail "vector varargs ${caller}->${callee} aborted"
            [[ $out == 167 ]] || fail "vector varargs ${caller}->${callee} printed $out, expected 167"
            echo "PASS: vector varargs cross-ABI (${caller} caller, ${callee} consumer, -O1)"
        done
    done
fi

# Optimisation sweep on the self leg: the caller census (%al) and the fetch
# shapes must not drift with the pipeline.
for opt in -O0 -O2 -O3; do
    "$ccc" "$opt" -c "$tmp/consumer.c" -o "$tmp/c_$opt.o"
    "$ccc" "$opt" -c "$tmp/driver.c" -o "$tmp/d_$opt.o"
    bin="$tmp/x$opt"
    "$gcc_bin" -O1 -o "$bin" "$tmp/d_$opt.o" "$tmp/c_$opt.o"
    out=$("$bin") || fail "self leg $opt aborted"
    [[ $out == 167 ]] || fail "self leg $opt printed $out, expected 167"
    echo "PASS: vector varargs self leg $opt"
done

# ------------------------------------------------- LCCC ext_vector fixture --
# Clang-shaped carriers: SSE single-slot (u32x3 padded 16, u64x1 8),
# INTEGER GP (u8x3 padded 4, bit-packed bool4), and the argument stream
# continuing intact after each fetch.
cat > "$tmp/ext.c" <<'EOF'
#include <stdarg.h>
#include <stdio.h>
typedef unsigned long long u64x1 __attribute__((ext_vector_type(1)));
typedef unsigned int u32x3 __attribute__((ext_vector_type(3)));
typedef unsigned char u8x3 __attribute__((ext_vector_type(3)));
typedef float f32x4 __attribute__((ext_vector_type(4)));
typedef _Bool bool4 __attribute__((ext_vector_type(4)));
static long long consumer(int n, ...) {
    va_list ap;
    va_start(ap, n);
    u32x3 a = va_arg(ap, u32x3);
    long l = va_arg(ap, long);
    u64x1 b = va_arg(ap, u64x1);
    double d = va_arg(ap, double);
    u8x3 c = va_arg(ap, u8x3);
    int i = va_arg(ap, int);
    f32x4 f = va_arg(ap, f32x4);
    bool4 bo = va_arg(ap, bool4);
    va_end(ap);
    return (long long)a[0] + a[2] + l + b[0] + (long long)d + c[0] + c[2] + i
         + (long long)(f[0] + f[3]) + bo[0] + bo[3] + n;
}
/* Indirect (function-pointer) variadic call: the same classification must
 * survive the fptr argument path, not only direct calls. */
static long long (*indirect)(int, ...) = consumer;
int main(void) {
    u32x3 a = {10, 20, 30};
    u64x1 b = {100};
    u8x3 c = {7, 8, 9};
    f32x4 f = {1.0f, 0, 0, 2.0f};
    bool4 bo = {1, 0, 1, 1};
    long long r1 = consumer(5, a, 1000L, b, 0.5, c, 3, f, bo);
    long long r2 = indirect(5, a, 1000L, b, 0.5, c, 3, f, bo);
    long long want = 10 + 30 + 1000 + 100 + 0 + 7 + 9 + 3 + 3 + 1 + 1 + 5;
    if (r1 != want) {
        printf("direct leg: %lld != %lld\n", r1, want);
        return 1;
    }
    if (r2 != want) {
        printf("indirect leg: %lld != %lld\n", r2, want);
        return 1;
    }
    puts("ok");
    return 0;
}
EOF

for opt in -O0 -O1 -O2 -O3; do
    "$ccc" "$opt" "$tmp/ext.c" -o "$tmp/ext$opt" || fail "ext_vector leg $opt"
    out=$("$tmp/ext$opt") || fail "ext_vector leg $opt aborted"
    [[ $out == ok ]] || fail "ext_vector leg $opt printed $out"
    echo "PASS: ext_vector varargs round-trip $opt"
done

echo "PASS vector varargs ABI contract"
