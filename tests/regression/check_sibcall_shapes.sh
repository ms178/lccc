#!/usr/bin/env bash
# Sibling-call conversion (TODO-3 / §5b of the codegen-gaps ledger).
#
# A call that is the last instruction of its block, whose result is returned
# verbatim by the block's Return (or whose result is dead under Return(None)),
# with zero stack-passed arguments, no used alloca, no dynamic alloca, no
# inline asm, non-variadic, not sret, is converted to
# `<arg setup>; <epilogue>; jmp target` — the §5b soundness core: with no
# stack-passed arguments the callee reads nothing below %rsp, so the 16-byte
# window shift between `call` and `jmp` cannot change any byte it observes
# (pr23324 stays impossible by construction).
#
# This gate pins:
#   * the converted x86 shapes (fp, integer, void-forwarder, indirect) —
#     `jmp target` present, `call target`/`ret` absent in that body;
#   * the refusals the design demands, asserted AS refusals:
#     the 7-stack-argument forwarder (pr23324's shape), the variadic
#     forwarder, the address-taken-local forwarder, and the dynamic-alloca
#     user — each must keep `call` + `ret`;
#   * the kill switch (CCC_NO_SIBCALL=1 restores `call` + `ret` everywhere);
#   * runtime parity: every positive shape, run with the conversion on and
#     off, must produce identical output (the conversion may not change a
#     single observable result);
#   * the §5.3 oracle shape: GCC 16.2 compiles `dmin2` to `jmp fmin`;
#     lccc must match it (when the gcc-16 oracle is on PATH).
set -euo pipefail
CCC=${LCCC_BIN:-${CCC:-target/fastbuild/lccc}}
tmp=${TMPDIR:-/tmp}/lccc-sibcall.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"

cat > "$tmp/shapes.c" <<'C'
#include <stdio.h>
#include <math.h>

__attribute__((noinline)) long sink7(long a,long b,long c,long d,long e,long f,long g)
{ return a+b+c+d+e+f+g; }
__attribute__((noinline)) int g6(int a,int b,int c,int d,int e,int f)
{ return a+b+c+d+e+f; }
__attribute__((noinline)) int g6p(int *p,int a,int b,int c,int d,int e)
{ return p[0] + p[1] + a+b+c+d+e; }
__attribute__((noinline)) int vsum(int n, ...) { return n; }

/* §5.3: the textbook fp forwarder. */
double dmin2(double x, double y) { return __builtin_fmin(x, y); }
double dmax2(double x, double y) { return __builtin_fmax(x, y); }
/* Integer forwarder. */
int iw(int a, int b) { return g6(a, b, 0, 0, 0, 0); }
/* Void forwarder with a discarded result. */
void vfwd(void) { puts("hi"); }
/* Indirect forwarder: the tail target rides in a register. */
typedef int (*fp6)(int,int,int,int,int,int);
int icall(fp6 g, int a, int b, int c, int d, int e, int f)
{ return g(a, b, c, d, e, f); }

/* Refusals the design demands, each asserted AS a refusal. */
long fwd7(long v) { return sink7(v, v, v, v, v, v, 77L); }      /* 7th arg on stack (pr23324) */
int vwrap(const char *s) { return printf("%s", s); }            /* variadic callee */
int addr_taken(int x) {                                        /* &local reaches the call */
    int t[2]; t[0] = x; t[1] = x + 1;
    return g6p(t, x, 0, 0, 0, 0);
}
int dyn_alloca(int n) {                                        /* dynamic stack below %rsp */
    int *p = __builtin_alloca(n);
    p[0] = n;
    return g6(p[0], n, 0, 0, 0, 0);
}
C

run_shapes() { # $1 = out suffix; env prefix in $2 ("" or "1")
    if [ "$2" = "1" ]; then
        CCC_NO_SIBCALL=1 "$CCC" -O2 -S "$tmp/shapes.c" -o "$tmp/shapes$1.s"
    else
        "$CCC" -O2 -S "$tmp/shapes.c" -o "$tmp/shapes$1.s"
    fi
}
run_shapes "" ""
run_shapes "_ctl" "1"

body() { # $1 asm, $2 label; body text from label to end of function
    sed -n "/^$2:/,/^\.size/p" "$1" | sed '/^\.size/q'
}
has()  { grep -q -e "$2" <<<"$1"; }
lacks() { if grep -q -e "$2" <<<"$1"; then echo "unexpected '$2' in: $1" >&2; return 1; fi; }

# --- Positive shapes: converted, no call, no ret in the body ---
for fn in dmin2 dmax2 iw vfwd icall; do
    b=$(body "$tmp/shapes.s" "$fn")
    case "$fn" in
        dmin2) want='jmp fmin@PLT' ;;
        dmax2) want='jmp fmax@PLT' ;;
        iw)    want='jmp g6@PLT' ;;
        vfwd)  want='jmp puts@PLT' ;;
        icall) want='jmpq \*%r10' ;;
    esac
    has "$b" "$want" || { echo "$fn: missing '$want':"; echo "$b" >&2; exit 1; }
    lacks "$b" '^[[:space:]]*call ' || { echo "$fn: tail still calls"; exit 1; }
    lacks "$b" '^[[:space:]]*ret$' || { echo "$fn: tail still rets"; exit 1; }
done

# --- Refusals: asserted AS refusals (call + ret survive, no jmp out) ---
for fn in fwd7 vwrap addr_taken dyn_alloca; do
    b=$(body "$tmp/shapes.s" "$fn")
    has "$b" '^[[:space:]]*call ' || { echo "$fn: refusal converted a site it must not:"; echo "$b" >&2; exit 1; }
    has "$b" '^[[:space:]]*ret$' || { echo "$fn: refusal lost its ret:"; echo "$b" >&2; exit 1; }
done
# fwd7 specifically: the 7th argument is materialised on the stack — the
# pr23324 window-shift hazard the zero-stack-argument guard exists to refuse.
b=$(body "$tmp/shapes.s" fwd7)
has "$b" 'call sink7@PLT' || { echo "fwd7: not the sink7 call"; exit 1; }

# --- Kill switch: every positive site reverts to call + ret ---
for fn in dmin2 iw vfwd icall; do
    b=$(body "$tmp/shapes_ctl.s" "$fn")
    has "$b" '^[[:space:]]*call ' || { echo "$fn (control): kill switch lost the call"; exit 1; }
    has "$b" '^[[:space:]]*ret$' || { echo "$fn (control): kill switch lost the ret"; exit 1; }
done

# --- Runtime parity: conversion on vs off must be byte-identical, and
# --- the values must be right (tail results are real results).
cat > "$tmp/rt.c" <<'C'
#include <stdio.h>
#include <math.h>
#include <string.h>
__attribute__((noinline)) int g6(int a,int b,int c,int d,int e,int f)
{ return a+b+c+d+e+f; }
typedef int (*fp6)(int,int,int,int,int,int);
int icall(fp6 g, int a, int b, int c, int d, int e, int f)
{ return g(a, b, c, d, e, f); }
double dmin2(double x, double y) { return __builtin_fmin(x, y); }
double dmax2(double x, double y) { return __builtin_fmax(x, y); }
void vfwd(void) { puts("hi"); }
int iw_call(void);
static int gi(int a,int b,int c,int d,int e,int f){ return g6(a,b,c,d,e,f); }
int main(void) {
    vfwd();
    double z = dmin2(3.5, -2.5) + dmax2(3.5, -2.5);
    int r = iw_call();
    printf("%.6f %d %d %d\n", z, r, icall(g6, 1, 2, 3, 4, 5, 6), gi(1,2,3,4,5,6));
    return 0;
}
int iw_call(void) { return g6(1, 2, 0, 0, 0, 0); }
C
"$CCC" -O2 "$tmp/rt.c" -o "$tmp/rt_on"
CCC_NO_SIBCALL=1 "$CCC" -O2 "$tmp/rt.c" -o "$tmp/rt_off"
on=$("$tmp/rt_on")
off=$("$tmp/rt_off")
if [ "$on" != "$off" ]; then
    echo "runtime parity broken:"; echo "on : $on"; echo "off: $off"; exit 1
fi
case "$on" in
    *"hi"*"1.000000"*"3"*"21"*) ;;
    *) echo "unexpected runtime values: $on"; exit 1 ;;
esac

# --- §5.3 oracle: GCC 16.2 compiles dmin2 to `jmp fmin`; match it ---
gcc=${GCC:-gcc-16}
if "$gcc" --version 2>/dev/null | grep -c "16\.2" >/dev/null; then
    printf 'double dmin2(double x, double y) { return __builtin_fmin(x, y); }\n' > "$tmp/o.c"
    o=$("$gcc" -O2 -S "$tmp/o.c" -o - | sed -n "/^dmin2:/,/^\.size/p" | sed '/^\.size/q')
    if ! grep -q 'jmp.*fmin' <<<"$o"; then
        echo "note: gcc-16.2 did not sibcall dmin2 here (skipping shape pin)"
    else
        b=$(body "$tmp/shapes.s" dmin2)
        has "$b" 'jmp fmin@PLT' || { echo "lccc shape diverges from gcc-16.2"; exit 1; }
    fi
else
    echo "SKIP: gcc-16.2 oracle not on PATH (shape parity leg)"
fi

# --- Adversarial per-site and ABI contracts ---------------------------------
# The positive cases above are deliberately simple.  These cases exercise the
# states that a block-wide hint or a partial ABI classifier misses: two void
# IR calls at different instruction indices, constants in the two aligned
# stack-argument classes, returns-twice callees, both retpoline modes, and
# outgoing YMM/ZMM arguments with dirty upper lanes.
cat > "$tmp/instrumented.c" <<'C'
void __cyg_profile_func_enter(void *this_fn, void *call_site);
void __cyg_profile_func_exit(void *this_fn, void *call_site);
void observed(void) {}
C
"$CCC" -O2 -finstrument-functions -S "$tmp/instrumented.c" -o "$tmp/instrumented.s"
b=$(body "$tmp/instrumented.s" observed)
has "$b" 'call __cyg_profile_func_enter@PLT' || {
    echo "instrumented per-site identity: entry hook was suppressed or moved" >&2
    echo "$b" >&2
    exit 1
}
has "$b" 'jmp __cyg_profile_func_exit@PLT' || {
    echo "instrumented per-site identity: final exit hook was not the tail site" >&2
    echo "$b" >&2
    exit 1
}
lacks "$b" 'jmp __cyg_profile_func_enter@PLT' || {
    echo "instrumented per-site identity: entry hook aliases the final void site" >&2
    exit 1
}
lacks "$b" 'call __cyg_profile_func_exit@PLT' || {
    echo "instrumented per-site identity: exit hook emitted twice" >&2
    exit 1
}

cat > "$tmp/abi.c" <<'C'
typedef __int128 i128;
typedef float V32 __attribute__((vector_size(32)));
typedef float V64 __attribute__((vector_size(64)));

long double ld_target(long double x);
long double ld_const_fwd(void) { return ld_target(1.25L); }
i128 i128_target(long a, long b, long c, long d, long e, i128 x);
i128 i128_const_fwd(void) { return i128_target(1, 2, 3, 4, 5, 6); }
int _setjmp(void *env);
int returns_twice_fwd(void *env) { return _setjmp(env); }

typedef int (*fp6)(int, int, int, int, int, int);
int indirect(fp6 f, int a, int b, int c, int d, int e, int f6) {
    return f(a, b, c, d, e, f6);
}

V32 global32, global32_out;
V64 global64, global64_out;
long avx32_target(V32 x);
long avx64_target(V64 x);
long avx32_dirty_global_fwd(void) {
    global32_out = global32 + global32;
    return avx32_target(global32);
}
long avx64_dirty_global_fwd(void) {
    global64_out = global64 + global64;
    return avx64_target(global64);
}
C
"$CCC" -O2 -S "$tmp/abi.c" -o "$tmp/abi.s"
for fn_target in 'ld_const_fwd ld_target' 'i128_const_fwd i128_target' \
                 'returns_twice_fwd _setjmp'; do
    set -- $fn_target
    fn=$1
    target=$2
    b=$(body "$tmp/abi.s" "$fn")
    has "$b" "call $target@PLT" || {
        echo "$fn: $target must remain an ordinary call" >&2
        echo "$b" >&2
        exit 1
    }
    has "$b" '^[[:space:]]*ret$' || {
        echo "$fn: ordinary-call refusal lost its return" >&2
        exit 1
    }
    lacks "$b" "jmp $target@PLT" || {
        echo "$fn: unsafe sibling conversion" >&2
        exit 1
    }
done

for mode in thunk-extern thunk-inline; do
    "$CCC" -O2 "-mindirect-branch=$mode" -S "$tmp/abi.c" -o "$tmp/$mode.s"
    b=$(body "$tmp/$mode.s" indirect)
    lacks "$b" 'jmp \*%r10' || {
        echo "$mode: sibling call bypassed the indirect-branch thunk" >&2
        exit 1
    }
    if [ "$mode" = thunk-extern ]; then
        has "$b" 'jmp __x86_indirect_thunk_r10' || {
            echo "$mode: missing external tail thunk" >&2
            echo "$b" >&2
            exit 1
        }
    else
        has "$b" 'call \.Lrpl_set_' || {
            echo "$mode: missing inline tail-retpoline sequence" >&2
            echo "$b" >&2
            exit 1
        }
        has "$b" 'movq %r10, (%rsp)' || {
            echo "$mode: inline sequence does not dispatch through r10" >&2
            exit 1
        }
    fi
done

for width in 32 64; do
    isa=-mavx2
    [ "$width" = 64 ] && isa=-mavx512f
    "$CCC" -O2 "$isa" -S "$tmp/abi.c" -o "$tmp/abi$width.s"
    b=$(body "$tmp/abi$width.s" "avx${width}_dirty_global_fwd")
    has "$b" "jmp avx${width}_target@PLT" || {
        echo "avx$width: expected a safe register-only sibling call" >&2
        echo "$b" >&2
        exit 1
    }
    lacks "$b" 'vzeroupper' || {
        echo "avx$width: epilogue zeroes live outgoing upper vector lanes" >&2
        exit 1
    }
done

# --- Runtime witnesses for every repaired ABI class -------------------------
cat > "$tmp/abi_runtime.c" <<'C'
#include <stdio.h>
typedef __int128 i128;
typedef float V32 __attribute__((vector_size(32)));
typedef float V64 __attribute__((vector_size(64)));

__attribute__((noinline)) long double ld_target(long double x) { return x + 2.5L; }
long double ld_const_fwd(void) { return ld_target(1.25L); }
__attribute__((noinline)) i128 i128_target(long a, long b, long c, long d, long e, i128 x) {
    return a + b + c + d + e + x;
}
i128 i128_const_fwd(void) { return i128_target(1, 2, 3, 4, 5, 6); }

V32 global32, global32_out;
__attribute__((noinline)) long avx32_target(V32 x) {
    long mask = 0;
    for (int i = 0; i < 8; ++i) if (x[i] == global32[i]) mask |= 1L << i;
    return mask;
}
long avx32_dirty_global_fwd(void) {
    global32_out = global32 + global32;
    return avx32_target(global32);
}
#ifdef RUN64
V64 global64, global64_out;
__attribute__((noinline)) long avx64_target(V64 x) {
    long mask = 0;
    for (int i = 0; i < 16; ++i) if (x[i] == global64[i]) mask |= 1L << i;
    return mask;
}
long avx64_dirty_global_fwd(void) {
    global64_out = global64 + global64;
    return avx64_target(global64);
}
#endif

int main(void) {
    unsigned long long ld_ok = ld_const_fwd() == 3.75L;
    unsigned long long i128_bits = (unsigned long long)i128_const_fwd();
    for (int i = 0; i < 8; ++i) global32[i] = (float)(i + 1);
    long mask32 = avx32_dirty_global_fwd();
#ifdef RUN64
    for (int i = 0; i < 16; ++i) global64[i] = (float)(i + 1);
    long mask64 = avx64_dirty_global_fwd();
#else
    long mask64 = 0xffff;
#endif
    printf("%llu %llu %lx %lx\n", ld_ok, i128_bits, mask32, mask64);
    return !(ld_ok == 1 && i128_bits == 21 && mask32 == 0xff && mask64 == 0xffff);
}
C
for width in 32 64; do
    isa=-mavx2
    extra=()
    if [ "$width" = 64 ]; then
        isa=-mavx512f
        extra=(-DRUN64)
        grep -qw avx512f /proc/cpuinfo 2>/dev/null || {
            echo "note: avx512f runtime leg skipped (host CPU lacks AVX-512F)"
            continue
        }
    fi
    "$CCC" -O2 "$isa" "${extra[@]}" "$tmp/abi_runtime.c" -o "$tmp/abi${width}_on"
    CCC_NO_SIBCALL=1 "$CCC" -O2 "$isa" "${extra[@]}" "$tmp/abi_runtime.c" -o "$tmp/abi${width}_off"
    on=$(timeout 10 "$tmp/abi${width}_on") || {
        echo "avx$width runtime failed with sibling calls enabled" >&2
        exit 1
    }
    off=$(timeout 10 "$tmp/abi${width}_off") || {
        echo "avx$width runtime failed with sibling calls disabled" >&2
        exit 1
    }
    [ "$on" = "$off" ] && [ "$on" = '1 21 ff ffff' ] || {
        echo "avx$width runtime mismatch: on='$on' off='$off' (want '1 21 ff ffff')" >&2
        exit 1
    }
    echo "PASS: avx$width sibling-call runtime and aligned stack arguments"
done

cat > "$tmp/instrumented_runtime.c" <<'C'
#include <stdio.h>
static int events[2];
static int count;
__attribute__((no_instrument_function)) void __cyg_profile_func_enter(void *a, void *b) {
    (void)a; (void)b; events[count++] = 1;
}
__attribute__((no_instrument_function)) void __cyg_profile_func_exit(void *a, void *b) {
    (void)a; (void)b; events[count++] = 2;
}
void observed(void) {}
__attribute__((no_instrument_function)) int main(void) {
    observed();
    printf("%d%d\n", events[0], events[1]);
    return !(count == 2 && events[0] == 1 && events[1] == 2);
}
C
"$CCC" -O2 -finstrument-functions "$tmp/instrumented_runtime.c" -o "$tmp/instrumented_on"
CCC_NO_SIBCALL=1 "$CCC" -O2 -finstrument-functions "$tmp/instrumented_runtime.c" -o "$tmp/instrumented_off"
on=$("$tmp/instrumented_on")
off=$("$tmp/instrumented_off")
[ "$on" = "$off" ] && [ "$on" = 12 ] || {
    echo "instrumented runtime order mismatch: on='$on' off='$off' (want '12')" >&2
    exit 1
}
echo "PASS: per-instruction void-call identity and runtime hook order"

# The shared driver computes hints on every target, but only x86-64 implements
# conversion. Other backends must not grow a silent partial tail-call path.
cat > "$tmp/cross.c" <<'C'
extern int target(int);
int forward(int x) { return target(x); }
C
bindir=$(dirname "$CCC")
for target_cc in lccc-i686 lccc-arm lccc-riscv; do
    [ -x "$bindir/$target_cc" ] || {
        echo "missing cross-target compiler: $bindir/$target_cc" >&2
        exit 1
    }
    "$bindir/$target_cc" -O2 -S "$tmp/cross.c" -o "$tmp/$target_cc.on.s"
    CCC_NO_SIBCALL=1 "$bindir/$target_cc" -O2 -S "$tmp/cross.c" -o "$tmp/$target_cc.off.s"
    cmp -s "$tmp/$target_cc.on.s" "$tmp/$target_cc.off.s" || {
        echo "$target_cc: unsupported sibling-call path changed target assembly" >&2
        diff -u "$tmp/$target_cc.off.s" "$tmp/$target_cc.on.s" >&2
        exit 1
    }
    echo "PASS: $target_cc leaves sibling-call lowering disabled"
done

echo "sibcall-shapes gate OK"
