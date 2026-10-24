#!/usr/bin/env bash
# GCC `vector_size` alignment model and the >16-byte MEMORY-class argument
# convention, measured against the GCC 16 oracle in both call directions.
#
# GCC gives a vector type TYPE_ALIGN == its size on x86 (`layout_type`:
# "always naturally align vectors", uncapped by the x86 TARGET_VECTOR_ALIGNMENT
# hook). That number drives struct layout, object alignment, `__alignof__` and
# the parameter-passing alignment ("the ABI for passing parameters with
# 32-byte alignment has changed in GCC 4.6": a MEMORY-class argument sits at an
# address that is a multiple of its FULL alignment, and the caller realigns
# %rsp so the outgoing area base is too). Only C11 `_Alignof(type)` reports a
# smaller figure -- `min_align_of_type` caps non-user-aligned types at
# BIGGEST_ALIGNMENT: 16, or 32 under -mavx, 64 under -mavx512f.
#
# lccc used the `_Alignof` figure (16) as the type's alignment everywhere, so
# `struct { int z; v32 v; }` was 48 bytes with `v` at 16 (GCC: 64, at 32), a
# 32-byte vector local/global was 16-aligned, and a 32-byte vector argument
# was pushed at a 16-aligned %rsp while every callee (GCC, Clang, lccc's own
# prologue and va_arg) rounds the slot up to 32 -- half of all stack residues
# read 16 bytes of garbage (ABI-EXTVEC-VA-1 and its vector_size twin).
#
# Part 1  layout/alignof probes: `-S` both compilers over the flag matrix and
#         compare the emitted constants verbatim (compile-only, so -mavx512f
#         needs no AVX-512 host).
# Part 2  x86-64 runtime matrix: 32- and 64-byte vectors as variadic, as a
#         named argument after six GP arguments (24/56 bytes of slot padding)
#         and as the first stack argument, at -O0 and -O2, in all four
#         compiler pairings, on both stack residues mod 32.
# Part 3  i386 (-m32): the layout probes agree (Part 1), lccc agrees with
#         itself, and 16-byte vectors must agree with GCC through the modeled
#         XMM carrier. The 32/64-byte cross-compiler legs remain LATCHED as
#         divergent under ABI-GNUVEC-I386-1: GCC keeps those vectors at their
#         full alignment with a realigned %esp, while lccc's i686 call sequence
#         packs them 4-granular. A latched leg that changes -- fixed or newly
#         broken -- fails this gate, so the remaining debt is re-measured on
#         every run instead of hiding.
set -euo pipefail

root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
ccc=${CCC:-$root/target/fastbuild/lccc}
gate=check_gnu_vector_align_abi
# shellcheck source=tests/regression/lib/gcc_oracle.sh
. "$root/tests/regression/lib/gcc_oracle.sh"
require_gcc_oracle "$gate" || exit 1
gcc_bin=$GCC_ORACLE_BIN
[ -x "$ccc" ] || { echo "FAIL: $gate: compiler not built: $ccc" >&2; exit 1; }
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
fail=0
pass=0
ok() { pass=$((pass + 1)); echo "ok    $*"; }
bad() { fail=$((fail + 1)); echo "FAIL  $*"; }

# ---------------------------------------------------------------- Part 1 --
cat > "$tmp/probe.c" <<'EOF'
typedef unsigned T32 __attribute__((vector_size(32)));
typedef unsigned T64 __attribute__((vector_size(64)));
typedef unsigned T256 __attribute__((vector_size(256)));
typedef T32 AT32 __attribute__((aligned(32)));
typedef T64 AT8 __attribute__((aligned(8)));
typedef double D4 __attribute__((aligned(4)));
struct S { int z; T32 v; };
struct S64 { int z; T64 v; };
struct A { _Alignas(32) int x; };
struct N { struct A a; };
struct SN { struct S s; };
struct __attribute__((aligned(8))) SA8 { T32 v; };
struct __attribute__((aligned(64))) SA64 { T32 v; };
struct SF { T32 v __attribute__((aligned(8))); };
struct C { char c; AT8 v; };
struct D { char c; D4 d; };
union U { int i; T64 v; };
T32 x; struct S s; T64 arr[2]; D4 gd; AT8 gv;
int r[] = {
    _Alignof(T32), __alignof__(T32), _Alignof(T64), __alignof__(T64), _Alignof(T256), __alignof__(T256),
    sizeof(struct S), _Alignof(struct S), __alignof__(struct S), __builtin_offsetof(struct S, v),
    sizeof(struct S64), _Alignof(struct S64), __builtin_offsetof(struct S64, v),
    _Alignof(x), _Alignof(s), _Alignof(T32[2]), _Alignof(arr), _Alignof(AT32), _Alignof(AT8), __alignof__(AT8),
    _Alignof(struct N), _Alignof(struct SN), _Alignof(struct SA8), _Alignof(struct SA64), _Alignof(struct SF),
    _Alignof(union U), __alignof__(union U), sizeof(struct SF), sizeof(struct SA8),
    sizeof(struct C), __builtin_offsetof(struct C, v), _Alignof(struct C),
    sizeof(struct D), __builtin_offsetof(struct D, d), _Alignof(struct D), _Alignof(D4), __alignof__(D4),
    _Alignof(gd), _Alignof(gv),
};
EOF
constants() {  # constants <compiler> <flags...>: the `.long` data of `r` in probe.c
    local cc=$1; shift
    # Scope the extraction to the initialiser of `r` (from its label to the
    # first non-`.long` line).  A bare `.long` grep also picks up data the
    # toolchain appends on its own -- a GCC configured with
    # `--enable-cet`/default `-fcf-protection` (Ubuntu runners) emits a
    # `.note.gnu.property` of `.long 1f - 0f`/`.long 0xc0000002` entries
    # after the object, which is not an ABI constant and must not be compared.
    "$cc" "$@" -w -O0 -S "$tmp/probe.c" -o - \
        | awk '/^r:/ { in_r = 1; next }
               in_r && /^[[:space:]]*\.long[[:space:]]/ { sub(/^[[:space:]]*\.long[[:space:]]*/, ""); print; next }
               in_r { exit }' \
        | tr '\n' ' '
}
for flags in "-m64" "-m64 -mavx" "-m64 -mavx2" "-m64 -mavx512f" "-m32" "-m32 -mavx"; do
    # shellcheck disable=SC2086  # flags are a word list on purpose
    want=$(constants "$gcc_bin" $flags)
    # shellcheck disable=SC2086
    got=$(constants "$ccc" $flags) || got="<compile failed>"
    if [ -n "$want" ] && [ "$want" = "$got" ]; then
        ok "layout/alignof constants agree with GCC [$flags] ($(wc -w <<<"$want") values)"
    else
        bad "layout/alignof constants differ [$flags]"
        echo "      gcc : $want"
        echo "      lccc: $got"
    fi
done

# ------------------------------------------------------------ Part 2 & 3 --
cat > "$tmp/callee.c" <<'EOF'
#include <stdarg.h>
typedef unsigned T __attribute__((vector_size(LANES * 4)));
unsigned long long f(int n, ...) {
    va_list ap; va_start(ap, n);
    T v = va_arg(ap, T);
    int k = va_arg(ap, int);
    va_end(ap);
    return v[0] + v[LANES - 1] * 1000ull + k * 1000000ull;
}
unsigned long long g(int a, int b, int c, int d, int e, int ff, int extra, T v, int k) {
    return v[0] + v[LANES - 1] * 1000ull + k * 1000000ull + extra * 1000000000ull + a + b + c + d + e + ff;
}
unsigned long long h(T v, int k) { return v[0] + v[LANES - 1] * 1000ull + k * 1000000ull; }
EOF
cat > "$tmp/caller.c" <<'EOF'
typedef unsigned T __attribute__((vector_size(LANES * 4)));
unsigned long long f(int n, ...);
unsigned long long g(int a, int b, int c, int d, int e, int ff, int extra, T v, int k);
unsigned long long h(T v, int k);
__attribute__((noinline)) int run(int depth) {
    T v = {0}; v[0] = 1; v[LANES - 1] = 9;
    volatile char pad[8]; pad[0] = (char)depth;
    int bad = 0;
    if (f(2, v, 17) != 1 + 9 * 1000ull + 17 * 1000000ull) bad |= 1;
    if (g(1, 2, 3, 4, 5, 6, 7, v, 9) != 1 + 9 * 1000ull + 9 * 1000000ull + 7 * 1000000000ull + 21) bad |= 2;
    if (h(v, 5) != 1 + 9 * 1000ull + 5 * 1000000ull) bad |= 4;
    return bad + pad[0] - depth;
}
/* one extra 16-byte slot: the calls run on both residues of the stack mod 32 */
__attribute__((noinline)) int shift(int depth) { volatile char pad[16]; pad[0] = 0; return run(depth) + pad[0]; }
int main(void) { return run(0) | (shift(1) << 3); }
EOF

# build <bits> <opt> <lanes>: objects cl/el (lccc caller/callee), cg/eg (gcc).
build() {
    local bits=$1 opt=$2 lanes=$3 d="-DLANES=$3"
    "$ccc" "-m$bits" "$opt" "$d" -c "$tmp/caller.c" -o "$tmp/cl.o" &&
    "$ccc" "-m$bits" "$opt" "$d" -c "$tmp/callee.c" -o "$tmp/el.o" &&
    "$gcc_bin" "-m$bits" "$opt" -w "$d" -c "$tmp/caller.c" -o "$tmp/cg.o" &&
    "$gcc_bin" "-m$bits" "$opt" -w "$d" -c "$tmp/callee.c" -o "$tmp/eg.o"
}
# leg <bits> <caller-obj> <callee-obj> -> exit code of the linked program
leg() {
    local bits=$1
    "$gcc_bin" "-m$bits" -no-pie "$tmp/$2.o" "$tmp/$3.o" -o "$tmp/t" || return 97
    set +e
    i386_run "$bits" "$tmp/t"
    local rc=$?
    set -e
    return $rc
}
# i386_run <bits> <binary>: run natively, or under qemu-i386 for ELF32 when
# the host cannot execute it.
i386_runner=""
i386_run() {
    if [ "$1" = 32 ] && [ -n "$i386_runner" ]; then "$i386_runner" "$2"; else "$2"; fi
}
if ! "$gcc_bin" -m32 -O0 -w -DLANES=4 "$tmp/caller.c" "$tmp/callee.c" -o "$tmp/probe32" 2>/dev/null; then
    i386_ok=no
elif "$tmp/probe32" >/dev/null 2>&1; then
    i386_ok=yes
else
    i386_ok=no
    for cand in qemu-i386 qemu-i386-static; do
        if command -v "$cand" >/dev/null 2>&1 && "$cand" "$tmp/probe32" >/dev/null 2>&1; then
            i386_runner=$cand; i386_ok=yes; break
        fi
    done
fi

# Part 2: x86-64, every leg must agree (exit 0).
for lanes in 8 16; do
    for opt in -O0 -O2; do
        if ! build 64 "$opt" "$lanes"; then bad "x86-64 build v$((lanes * 4)) $opt"; continue; fi
        for pair in cl/el cl/eg cg/el cg/eg; do
            rc=0; leg 64 "${pair%/*}" "${pair#*/}" || rc=$?
            if [ "$rc" = 0 ]; then ok "x86-64 v$((lanes * 4)) $opt $pair"; else bad "x86-64 v$((lanes * 4)) $opt $pair exit $rc (bit mask: 1 va_arg, 2 named, 4 first; <<3 shifted)"; fi
        done
    done
done

# Part 3: i386. v16 crosses must agree; larger-vector debt stays latched.
if [ "$i386_ok" = yes ]; then
    for lanes in 4 8 16; do
        for opt in -O0 -O2; do
            if ! build 32 "$opt" "$lanes"; then bad "i386 build v$((lanes * 4)) $opt"; continue; fi
            rc=0; leg 32 cl el || rc=$?
            if [ "$rc" = 0 ]; then ok "i386 v$((lanes * 4)) $opt lccc/lccc"; else bad "i386 v$((lanes * 4)) $opt lccc/lccc exit $rc"; fi
            rc=0; leg 32 cg eg || rc=$?
            if [ "$rc" = 0 ]; then ok "i386 v$((lanes * 4)) $opt gcc/gcc (oracle sanity)"; else bad "i386 v$((lanes * 4)) $opt gcc/gcc exit $rc"; fi
            for pair in cl/eg cg/el; do
                rc=0; leg 32 "${pair%/*}" "${pair#*/}" || rc=$?
                if [ "$lanes" = 4 ]; then
                    if [ "$rc" = 0 ]; then
                        ok "i386 v16 $opt $pair"
                    else
                        bad "i386 v16 $opt $pair exit $rc (bit mask: 1 va_arg, 2 named, 4 first; <<3 shifted)"
                    fi
                elif [ "$rc" != 0 ] && [ "$rc" != 97 ]; then
                    ok "i386 v$((lanes * 4)) $opt $pair diverges (exit $rc) -- latched ABI-GNUVEC-I386-1"
                else
                    bad "i386 v$((lanes * 4)) $opt $pair exit $rc: the latched ABI-GNUVEC-I386-1 verdict changed; re-measure and update this gate with the fix"
                fi
            done
        done
    done
else
    echo "note: no i386 execution capability (gcc -m32 link or qemu-i386); Part 3 not measured" >&2
fi

# ---------------------------------------------------------------- Part 4 --
# x86-64 `long double` ext vector lanes against GCC's x87 conventions
# (ABI-EXTVEC-LD-1). Clang hands a vector return of class MEMORY to LLVM
# "direct" (`X86_64ABIInfo::getIndirectReturnResult`: a vector is not an
# aggregate), which scalarizes `<N x x86_fp80>` into x87 returns: one lane
# is `%st(0)` with a 16-byte memory argument -- exactly GCC's
# `struct { long double }` (X87 class) -- and two lanes are `%st(0)/%st(1)`
# -- exactly GCC's `_Complex long double` return. GCC compiles the struct /
# complex side, lccc the ext-vector side, linked both ways; the values must
# round-trip bit-exactly. (Arguments of the two-lane shape are not compared
# with the complex form: Clang passes them `byval align 32` with lanes 10
# bytes apart, GCC passes a complex at 16 with the parts 16 apart; the slot
# alignment is covered by Part 1/2 and the lane stride by
# ext_vector_long_double_layout.c.)
cat > "$tmp/ld_lccc.c" <<'EOF2'
typedef long double T1 __attribute__((ext_vector_type(1)));
typedef long double T2 __attribute__((ext_vector_type(2)));
T1 ident1(T1 a) { return a; }
T1 kfour1(int k, T1 a, T1 b, T1 c, T1 d, int j) { T1 r = d; r[0] = a[0] + b[0] * 2 + c[0] * 4 + d[0] * 8 + k + j; return r; }
long double sum1(int k, T1 a, long double x, T1 b) { return a[0] * 3 + x + b[0] * 7 + k; }
T2 ident2(T2 a) { return a; }
T2 mk2(long double x, long double y) { T2 r; r[0] = x * 2; r[1] = y + 1; return r; }
struct S1 { long double x; };
struct S1 g_ident1(struct S1 a);
struct S1 g_kfour1(int k, struct S1 a, struct S1 b, struct S1 c, struct S1 d, int j);
long double g_sum1(int k, struct S1 a, long double x, struct S1 b);
_Complex long double g_ident2(_Complex long double a);
_Complex long double g_mk2(long double x, long double y);
int lccc_calls_gcc(void) {
    struct S1 a = {1.25L}, b = {2.5L}, c = {4.0L}, d = {8.0L};
    int bad = 0;
    if (g_ident1(a).x != 1.25L) bad |= 1;
    if (g_kfour1(3, a, b, c, d, 5).x != 1.25L + 5.0L + 16.0L + 64.0L + 3 + 5) bad |= 2;
    if (g_sum1(2, a, 0.5L, b) != 1.25L * 3 + 0.5L + 2.5L * 7 + 2) bad |= 4;
    _Complex long double c = g_mk2(4.0L, 0.5L);
    if (__real__ c != 8.0L || __imag__ c != 1.5L) bad |= 8;
    c = g_ident2(c);
    if (__real__ c != 8.0L || __imag__ c != 1.5L) bad |= 16;
    return bad;
}
EOF2
cat > "$tmp/ld_gcc.c" <<'EOF2'
#include <complex.h>
struct S1 { long double x; };
struct S1 g_ident1(struct S1 a) { return a; }
struct S1 g_kfour1(int k, struct S1 a, struct S1 b, struct S1 c, struct S1 d, int j) { struct S1 r = d; r.x = a.x + b.x * 2 + c.x * 4 + d.x * 8 + k + j; return r; }
long double g_sum1(int k, struct S1 a, long double x, struct S1 b) { return a.x * 3 + x + b.x * 7 + k; }
_Complex long double g_ident2(_Complex long double a) { return a; }
_Complex long double g_mk2(long double x, long double y) { return (x * 2) + (y + 1) * I; }
struct S1 ident1(struct S1 a);
struct S1 kfour1(int k, struct S1 a, struct S1 b, struct S1 c, struct S1 d, int j);
long double sum1(int k, struct S1 a, long double x, struct S1 b);
_Complex long double ident2(_Complex long double a);
_Complex long double mk2(long double x, long double y);
int lccc_calls_gcc(void);
int main(void) {
    struct S1 a = {1.25L}, b = {2.5L}, c = {4.0L}, d = {8.0L};
    int bad = 0;
    if (ident1(a).x != 1.25L) bad |= 32;
    if (kfour1(3, a, b, c, d, 5).x != 1.25L + 5.0L + 16.0L + 64.0L + 3 + 5) bad |= 64;
    if (sum1(2, a, 0.5L, b) != 1.25L * 3 + 0.5L + 2.5L * 7 + 2) bad |= 128;
    _Complex long double r = mk2(4.0L, 0.5L);
    if (creall(r) != 8.0L || cimagl(r) != 1.5L) bad |= 256;
    r = ident2(r);
    if (creall(r) != 8.0L || cimagl(r) != 1.5L) bad |= 512;
    return bad | lccc_calls_gcc();
}
EOF2
for lo in -O0 -O2; do
    for go in -O0 -O2; do
        rc=0
        "$ccc" -m64 "$lo" -c "$tmp/ld_lccc.c" -o "$tmp/ldl.o" &&
            "$gcc_bin" -m64 "$go" -w -c "$tmp/ld_gcc.c" -o "$tmp/ldg.o" &&
            "$gcc_bin" -m64 -no-pie "$tmp/ldl.o" "$tmp/ldg.o" -o "$tmp/ldt" || rc=97
        if [ "$rc" = 0 ]; then
            set +e; "$tmp/ldt"; rc=$?; set -e
        fi
        if [ "$rc" = 0 ]; then
            ok "x86-64 long double ext vector lanes: lccc $lo <-> gcc $go (x87 struct / complex conventions)"
        else
            bad "x86-64 long double ext vector lanes: lccc $lo <-> gcc $go rc=$rc (bits: 1-16 lccc caller, 32-512 gcc caller, 97 build)"
        fi
    done
done

echo "$gate: $pass passed, $fail failed"
[ "$fail" = 0 ] || { echo "FAIL: $gate" >&2; exit 1; }
echo "PASS: $gate"
