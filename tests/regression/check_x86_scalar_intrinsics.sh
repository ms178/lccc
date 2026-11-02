#!/bin/sh
# Bundled scalar x86 intrinsics (bmiintrin.h, lzcntintrin.h, bmi2intrin.h,
# popcntintrin.h, ia32intrin.h): runtime semantics equal GCC on its own headers for every
# LCCC flag set, and the ISA flags select the single instructions.
#   CCC/LCCC = compiler under test; GCC (default gcc-16, else gcc).
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
src=$here/x86_scalar_intrin/scalar_intrin.c
cc=${LCCC:-${CCC:?set CCC or LCCC}}
gcc=${GCC:-$(command -v gcc-16 || command -v gcc)}
tmp=$(mktemp -d "${TMPDIR:-/tmp}/scalar_intrin.XXXXXX")
trap 'rm -rf "$tmp"' EXIT INT TERM
fail=0
note() { printf '%s\n' "$*"; }
bad() { note "FAIL: $*"; fail=1; }

cpu_has() { grep -qw "$1" /proc/cpuinfo 2>/dev/null; }
isa_run=1
for f in bmi1 bmi2 abm; do cpu_has "$f" || isa_run=0; done

"$gcc" -O2 -mbmi -mbmi2 -mlzcnt -mpopcnt "$src" -o "$tmp/ref" 2>"$tmp/gcc.err" \
    || { cat "$tmp/gcc.err"; bad "GCC oracle build"; exit 1; }
if [ "$isa_run" = 1 ]; then
    "$tmp/ref" >"$tmp/ref.out"
else
    "$gcc" -O0 "$src" -o "$tmp/ref0" 2>/dev/null && note "SKIP-ISA: host lacks bmi1/bmi2/lzcnt; oracle unavailable" && exit 0
fi

i=0
for opt in -O0 -O2; do
    for isa in "" "-mbmi -mlzcnt -mbmi2" "-march=x86-64-v3" "-march=x86-64-v3 -mno-bmi -mno-lzcnt"; do
        i=$((i + 1))
        # shellcheck disable=SC2086
        if ! "$cc" $opt $isa -Werror=implicit-function-declaration "$src" -o "$tmp/t$i" 2>"$tmp/e$i"; then
            cat "$tmp/e$i"; bad "build $opt $isa"; continue
        fi
        "$tmp/t$i" >"$tmp/o$i" || { bad "run $opt $isa"; continue; }
        if cmp -s "$tmp/o$i" "$tmp/ref.out"; then
            note "PASS: semantics $opt ${isa:-baseline}"
        else
            diff "$tmp/ref.out" "$tmp/o$i" | head -6; bad "semantics $opt ${isa:-baseline}"
        fi
    done
done

# i686: the same headers (64-bit-only forms compiled out) against GCC -m32.
# Skipped, not failed, where the host lacks a 32-bit GCC runtime (multilib).
if "$gcc" -m32 -O2 -mbmi -mbmi2 -mlzcnt -mpopcnt "$src" -o "$tmp/ref32" 2>/dev/null; then
    "$tmp/ref32" >"$tmp/ref32.out"
    for flags in "-O0" "-O2" "-O2 -mbmi -mlzcnt -mbmi2"; do
        i=$((i + 1))
        # shellcheck disable=SC2086
        if ! "$cc" -m32 $flags -Werror=implicit-function-declaration "$src" -o "$tmp/t$i" 2>"$tmp/e$i"; then
            cat "$tmp/e$i"; bad "build -m32 $flags"; continue
        fi
        if "$tmp/t$i" >"$tmp/o$i" && cmp -s "$tmp/o$i" "$tmp/ref32.out"; then
            note "PASS: semantics -m32 $flags"
        else
            diff "$tmp/ref32.out" "$tmp/o$i" | head -6; bad "semantics -m32 $flags"
        fi
    done
else
    note "SKIP-M32: no 32-bit GCC runtime (gcc-multilib)"
fi

# popcntintrin.h: lccc's portable form on every flag set against GCC
# -mpopcnt (GCC refuses the call without POPCNT; lccc computes the count).
pop=$here/x86_scalar_intrin/popcnt_intrin.c
for mode in -m64 -m32; do
    if ! "$gcc" $mode -O2 -mpopcnt "$pop" -o "$tmp/pref" 2>/dev/null; then
        note "SKIP-POPCNT$mode: no GCC $mode runtime"
        continue
    fi
    "$tmp/pref" >"$tmp/pref.out"
    for flags in "-O0" "-O2" "-O2 -mpopcnt" "-O2 -march=x86-64-v2"; do
        i=$((i + 1))
        # shellcheck disable=SC2086
        if "$cc" $mode $flags -Werror=implicit-function-declaration "$pop" -o "$tmp/t$i" 2>"$tmp/e$i" &&
            "$tmp/t$i" >"$tmp/o$i" && cmp -s "$tmp/o$i" "$tmp/pref.out"; then
            note "PASS: popcnt $mode $flags"
        else
            cat "$tmp/e$i"; bad "popcnt $mode $flags"
        fi
    done
done

# PDEP/PEXT inlined into a caller whose target("arch=x86-64") forbids
# BMI2: the same values as GCC's BMI2 build (GCC itself rejects the call),
# and no pdep/pext in those callers' text (whole bodies: only a global
# label ends a function, local `.L` loop labels do not).
pp=$here/x86_scalar_intrin/pdep_pext_target.c
"$gcc" -O2 -mbmi2 -DTARGET_BASELINE= "$pp" -o "$tmp/ppref" 2>"$tmp/pp.err" \
    || { cat "$tmp/pp.err"; bad "GCC pdep/pext oracle build"; }
"$tmp/ppref" >"$tmp/ppref.out"
for mode in -m64 -m32; do
    for flags in "-O0" "-O2" "-O0 -mbmi2" "-O2 -mbmi2" "-O2 -march=x86-64-v3"; do
        i=$((i + 1))
        [ "$mode" = -m32 ] && ! "$gcc" -m32 -O2 -mbmi2 -DTARGET_BASELINE= "$pp" -o "$tmp/ppref32" 2>/dev/null \
            && { note "SKIP-PDEP-PEXT-m32: no GCC -m32 runtime"; break; }
        [ "$mode" = -m32 ] && "$tmp/ppref32" >"$tmp/ppref.out"
        # shellcheck disable=SC2086
        if "$cc" $mode $flags -Werror=implicit-function-declaration "$pp" -o "$tmp/t$i" 2>"$tmp/e$i" &&
            "$tmp/t$i" >"$tmp/o$i" && cmp -s "$tmp/o$i" "$tmp/ppref.out"; then
            note "PASS: pdep/pext target $mode $flags"
        else
            cat "$tmp/e$i" "$tmp/o$i" 2>/dev/null; bad "pdep/pext target $mode $flags"
        fi
    done
done
for flags in "-mbmi2" "-march=x86-64-v3"; do
    # shellcheck disable=SC2086
    "$cc" -O2 $flags -S "$pp" -o "$tmp/pp.s"
    for f in base_dep32 base_ext32 base_dep64 base_ext64; do
        awk -v f="$f:" '$0==f{p=1;next} p&&/^[A-Za-z_][A-Za-z0-9_.]*:$/{p=0} p' "$tmp/pp.s" |
            grep -Eq '\b(pdep|pext)[lq]?\b' && bad "$f: pdep/pext under target(\"arch=x86-64\") $flags"
    done
    for f in dep32:pdep ext32:pext dep64:pdep ext64:pext; do
        awk -v f="${f%%:*}:" '$0==f{p=1;next} p&&/^[A-Za-z_][A-Za-z0-9_.]*:$/{p=0} p' "$tmp/pp.s" |
            grep -q "${f#*:}" || bad "${f%%:*}: no ${f#*:} under $flags"
    done
done
[ "$fail" = 0 ] && note "PASS: pdep/pext per-function selection"

# Instruction contracts: one instruction under the ISA flag, a correct
# zero-defined fallback without it, and never a call or a bit loop.
cat >"$tmp/k.c" <<'C'
#include <immintrin.h>
unsigned tz(unsigned x) { return _tzcnt_u32(x); }
unsigned lz(unsigned x) { return _lzcnt_u32(x); }
unsigned long long tzq(unsigned long long x) { return _tzcnt_u64(x); }
unsigned long long lzq(unsigned long long x) { return _lzcnt_u64(x); }
unsigned dep(unsigned x, unsigned m) { return _pdep_u32(x, m); }
unsigned long long ext(unsigned long long x, unsigned long long m) { return _pext_u64(x, m); }
unsigned rl(unsigned x, int n) { return _rotl(x, n); }
int pc(unsigned x) { return _mm_popcnt_u32(x); }
long long pcq(unsigned long long x) { return _mm_popcnt_u64(x); }
C
fn() { awk -v f="$1:" '$0==f{p=1;next} p&&/^[A-Za-z_.][A-Za-z0-9_.]*:$/{p=0} p' "$2"; }
"$cc" -O2 -mbmi -mlzcnt -mbmi2 -mpopcnt -S "$tmp/k.c" -o "$tmp/k.s"
"$cc" -O2 -S "$tmp/k.c" -o "$tmp/k0.s"
for pair in tz:tzcnt lz:lzcnt tzq:tzcnt lzq:lzcnt dep:pdep ext:pext rl:rol pc:popcnt pcq:popcnt; do
    f=${pair%%:*}; insn=${pair#*:}
    body=$(fn "$f" "$tmp/k.s")
    printf '%s\n' "$body" | grep -q "$insn" || bad "$f: no $insn under the ISA flags"
    printf '%s\n' "$body" | grep -Eq '\bcall|\bj[a-z]+ ' && bad "$f: call/branch under the ISA flags"
done
for f in tz lz tzq lzq rl pc pcq; do
    body=$(fn "$f" "$tmp/k0.s")
    printf '%s\n' "$body" | grep -Eq 'tzcnt|lzcnt|popcnt|rep[; ]*bsf' && bad "$f: BMI/LZCNT/POPCNT instruction on the baseline"
    printf '%s\n' "$body" | grep -q '\bcall' && bad "$f: call on the baseline"
done
[ "$fail" = 0 ] && note "PASS: instruction contracts"

[ "$fail" = 0 ] && note "check_x86_scalar_intrinsics: PASS" || { note "check_x86_scalar_intrinsics: FAIL"; exit 1; }
