#!/usr/bin/env bash
# AArch64 and RISC-V leaf, parameter and unwind contracts, asserted on the
# -O2 assembly (the executable halves are leaf_params_fp_domain_cross.c and
# the abi_dirty_upper interop fixture, run by check_switch_cross_execute.py).
#
# AArch64:
#  - A call-free leaf keeps parameters where they arrive (x4-x7 are homes)
#    and builds no frame, also when its ninth parameter is on the stack.
#  - AAPCS64 leaves the bits of x[n] above a narrow argument or result
#    unspecified, so every zero test of an `int` reads the W view (`cmp w4,
#    #0`, `cbz w0`), a 32-bit parameter is copied into a callee-saved home
#    with a 32-bit move, and an 8-bit one is extended before it is tested.
#  - The in-place zero-extension of a 32-bit argument kept in its arrival
#    register is dropped when no path reads bits 63:32.
#  - Selects with FP operands are `fcsel`, and a select of zero reads the
#    zero register (`csel w0, w1, wzr, ne`).
#  - Callee-saved registers get their `.cfi_offset` rows.
#  - Contracted packed multiply-adds are `fmla` (main executed the `brk #0`
#    of an unhandled operation), and multiplications by constants of the
#    forms (2^k +- 1) << m are shift-adds.
#  - The emitter's call annotation (`// lccc-call-reads`) never reaches the
#    output.
#  - FP negation is one `fneg` (no constant-pool XOR through x0); `fabs`,
#    `copysign` (`bif`) and `fma` (`fmadd`) are inline instructions in
#    frameless leaves, never libm calls.
# RISC-V:
#  - `x | 1u << n` is `sllw` + `or` + one `sext.w`, a returned `unsigned`
#    parameter is a bare `ret` (it arrives sign-extended), a variadic
#    function's CFA is s0 + 64 and saved callee-saved registers have rows.
#  - `fma`/`fmaf` are `fmadd.d`/`fmadd.s`, not calls into libm.
set -euo pipefail

CCC=${LCCC_BIN:-${CCC:-target/fastbuild/lccc}}
bindir=$(CDPATH= cd -- "$(dirname -- "$CCC")" && pwd)
ARM=$bindir/lccc-arm
RV=$bindir/lccc-riscv
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
fail=0
bad() {
    echo "FAIL: $*" >&2
    fail=1
}

# The instruction lines of function `$2` in assembly file `$1`.
body() {
    awk -v f="$2" '
        $0 == f ":" { on = 1; next }
        on && /^\.size/ { exit }
        on && /^[ \t]+[a-z]/ && $1 !~ /^\./ { sub(/^[ \t]+/, ""); print }
    ' "$1"
}
count() { body "$1" "$2" | wc -l | tr -d ' '; }
has() { grep -Eq -- "$3" <<<"$(body "$1" "$2")"; }

cat >"$tmp/a64.c" <<'EOF'
extern void g(void);
extern int ext_int(void);
int six(int a, int b, int c, int d, int e, int f) { return a + b * c - d + e * f; }
int nine(int a, int b, int c, int d, int e, int f, int g, int h, int i)
{
    return a - b + c - d + e - f + g - h + i * 3;
}
double pick(double a, double b, int k) { return k ? a : b; }
int dirty_sel(int a, int b, int c, int d, int k) { return k ? a : b; }
int dirty_br(int a, int b, int c, int d, int k)
{
    if (k)
        return a - c;
    return b * d;
}
int live_k(int a, int b, int k)
{
    g();
    return k ? a : b;
}
int ret_branch(void)
{
    if (ext_int())
        return 1;
    return 2;
}
int bool_sel(_Bool f, int x) { return f ? x : -x; }
int sel0(int k, int a) { return k ? a : 0; }
long mul5(long x) { return x * 5; }
long mul_m3(long x) { return x * -3; }
int mul24(int x) { return x * 24; }
extern int h1(int);
int call_one(int a, int b) { return h1(a) + b; }
void axpy_d(double *restrict y, const double *restrict x, double a, int n)
{
    for (int i = 0; i < n; i++)
        y[i] = a * x[i] + y[i];
}
void axpy_f(float *restrict y, const float *restrict x, float a, int n)
{
    for (int i = 0; i < n; i++)
        y[i] = a * x[i] + y[i];
}
double negd(double x) { return -x; }
float negf(float x) { return -x; }
double absd(double x) { return __builtin_fabs(x); }
double csd(double x, double y) { return __builtin_copysign(x, y); }
float csf(float x, float y) { return __builtin_copysignf(x, y); }
double fmad(double a, double b, double c) { return __builtin_fma(a, b, c); }
EOF

if [ -x "$ARM" ]; then
    a="$tmp/a64.s"
    "$ARM" -O2 -S "$tmp/a64.c" -o "$a"
    for f in six nine pick dirty_sel dirty_br bool_sel; do
        if has "$a" "$f" 'x29|x30'; then bad "AArch64 $f: a call-free leaf built a frame"; fi
    done
    n=$(count "$a" six)
    [ "$n" -le 4 ] || bad "AArch64 six: $n instructions (madd/sub/madd/ret expected)"
    has "$a" six '^madd w0, w4, w5, ' || bad "AArch64 six: e and f not read in x4/x5"
    if has "$a" six '^mov w[45], w[45]$'; then bad "AArch64 six: unobservable zero-extension kept"; fi
    has "$a" nine '^ldrsw x[0-9]+, \[sp' || bad "AArch64 nine: stack parameter not read at entry"
    has "$a" pick '^fcsel d[0-9]+, ' || bad "AArch64 pick: select of doubles is not fcsel"
    has "$a" pick '^cmp w0, #0' || bad "AArch64 pick: int condition not tested in its W view"
    has "$a" dirty_sel '^cmp w4, #0' || bad "AArch64 dirty_sel: x4 condition not tested in its W view"
    if has "$a" dirty_sel 'x4'; then bad "AArch64 dirty_sel: the X view of an int argument was read"; fi
    n=$(count "$a" dirty_sel)
    [ "$n" -le 3 ] || bad "AArch64 dirty_sel: $n instructions (cmp/csel/ret expected)"
    has "$a" dirty_br '^cbz w4, ' || bad "AArch64 dirty_br: branch on an int argument reads x4"
    if has "$a" dirty_br '^mov w4, w4$'; then bad "AArch64 dirty_br: unobservable zero-extension kept"; fi
    has "$a" live_k '^mov w(19|2[0-8]), w2$' || bad "AArch64 live_k: int argument pre-stored with a 64-bit move"
    if has "$a" live_k '^mov x(19|2[0-8]), x2$'; then bad "AArch64 live_k: 64-bit copy of an int argument"; fi
    has "$a" live_k '^cmp w(19|2[0-8]), #0' || bad "AArch64 live_k: callee-saved condition not tested in its W view"
    if has "$a" ret_branch '^cbn?z x'; then bad "AArch64 ret_branch: int result tested in its X view"; fi
    has "$a" ret_branch '^cbn?z w[0-9]+, ' || bad "AArch64 ret_branch: int result not tested in its W view"
    has "$a" bool_sel '#0xff' || bad "AArch64 bool_sel: _Bool argument tested without masking its byte"
    if has "$a" bool_sel '^cmp x'; then bad "AArch64 bool_sel: _Bool tested in its X view"; fi
    grep -Eq '^[[:space:]]*\.cfi_offset x(19|2[0-8]), -[0-9]+' "$a" ||
        bad "AArch64 live_k: no .cfi_offset row for a saved callee-saved register"
    has "$a" sel0 '^csel w0, w1, wzr, ne$' || bad "AArch64 sel0: zero not read from wzr"
    for f in mul5 mul_m3 mul24; do
        if has "$a" "$f" '^(mul|madd|mov [wx][0-9]+, #)'; then bad "AArch64 $f: multiplication by a constant not strength-reduced"; fi
    done
    [ "$(count "$a" mul5)" -le 2 ] || bad "AArch64 mul5: add x0, x0, x0, lsl #2 + ret expected"
    for f in axpy_d axpy_f; do
        if has "$a" "$f" '^brk '; then bad "AArch64 $f: packed FMA reached an unhandled-operation trap"; fi
    done
    has "$a" axpy_d '^fmla v[0-9]+\.2d, ' || bad "AArch64 axpy_d: no packed fmla"
    has "$a" axpy_f '^fmla v[0-9]+\.4s, ' || bad "AArch64 axpy_f: no packed fmla"
    has "$a" call_one '^bl h1$' || bad "AArch64 call_one: the call is not a bare bl"
    for f in negd negf absd csd csf fmad; do
        if has "$a" "$f" 'x29|x30|^bl '; then bad "AArch64 $f: FP sign/fused op built a frame or called out"; fi
    done
    has "$a" negd '^fneg d[0-9]+, d[0-9]+$' || bad "AArch64 negd: negation is not one fneg"
    has "$a" negf '^fneg s[0-9]+, s[0-9]+$' || bad "AArch64 negf: negation is not one fneg"
    if has "$a" negd '^(eor|ldr|adrp) '; then bad "AArch64 negd: negation through a constant-pool XOR"; fi
    has "$a" absd '^fabs d[0-9]+, ' || bad "AArch64 absd: no fabs"
    has "$a" csd '^bif v[0-9]+\.8b, ' || bad "AArch64 csd: copysign is not a bif select"
    has "$a" csf '^bif v[0-9]+\.8b, ' || bad "AArch64 csf: copysign is not a bif select"
    has "$a" fmad '^fmadd d[0-9]+, ' || bad "AArch64 fmad: fma is not fmadd"
    if grep -q 'lccc-call-reads' "$a"; then bad "AArch64: call annotation leaked into the output"; fi
else
    echo "note: $ARM is absent; AArch64 leg skipped"
fi

cat >"$tmp/rv.c" <<'EOF'
unsigned bset32(unsigned x, unsigned n) { return x | (1u << n); }
unsigned lccc_pass(unsigned x) { return x; }
extern void g(long);
long vsum(int n, ...)
{
    __builtin_va_list ap;
    __builtin_va_start(ap, n);
    long s = 0;
    for (int i = 0; i < n; i++)
        s += __builtin_va_arg(ap, long);
    __builtin_va_end(ap);
    return s;
}
long keep(long a)
{
    g(a);
    g(a + 1);
    return a;
}
double fmad(double a, double b, double c) { return __builtin_fma(a, b, c); }
float fmaf_(float a, float b, float c) { return __builtin_fmaf(a, b, c); }
EOF

if [ -x "$RV" ]; then
    r="$tmp/rv.s"
    "$RV" -O2 -S "$tmp/rv.c" -o "$r"
    n=$(count "$r" bset32)
    [ "$n" -le 5 ] || bad "RISC-V bset32: $n instructions (li/sllw/or/sext.w/ret expected)"
    has "$r" bset32 '^sllw ' || bad "RISC-V bset32: shift not done in the W domain"
    [ "$(body "$r" lccc_pass)" = "ret" ] || bad "RISC-V lccc_pass: a sign-extended unsigned argument was re-extended"
    awk '$0 == "vsum:" { on = 1 } on && /\.cfi_def_cfa s0, 64/ { ok = 1 } on && /^\.size/ { exit } END { exit !ok }' "$r" ||
        bad "RISC-V vsum: variadic CFA is not s0 + 64"
    awk '$0 == "keep:" { on = 1 } on && /\.cfi_offset s1, -[0-9]+/ { ok = 1 } on && /^\.size/ { exit } END { exit !ok }' "$r" ||
        bad "RISC-V keep: no .cfi_offset row for saved s1"
    has "$r" fmad '^fmadd\.d ' || bad "RISC-V fmad: fma is not fmadd.d"
    has "$r" fmaf_ '^fmadd\.s ' || bad "RISC-V fmaf_: fmaf is not fmadd.s"
    for f in fmad fmaf_; do
        if has "$r" "$f" '^call '; then bad "RISC-V $f: fused multiply-add became a libm call"; fi
    done
else
    echo "note: $RV is absent; RISC-V leg skipped"
fi

if [ "$fail" -ne 0 ]; then
    exit 1
fi
echo "OK arm-riscv-leaf-contracts"
