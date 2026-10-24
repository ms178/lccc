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
        icall) want='jmp \*%r10' ;;
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
if "$gcc" --version 2>/dev/null | grep -q "16\.2"; then
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

echo "sibcall-shapes gate OK"
