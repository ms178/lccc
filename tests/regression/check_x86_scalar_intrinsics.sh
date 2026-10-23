#!/bin/sh
# Bundled scalar x86 intrinsics (bmiintrin.h, lzcntintrin.h, bmi2intrin.h,
# ia32intrin.h): runtime semantics equal GCC on its own headers for every
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
C
fn() { awk -v f="$1:" '$0==f{p=1;next} p&&/^[A-Za-z_.][A-Za-z0-9_.]*:$/{p=0} p' "$2"; }
"$cc" -O2 -mbmi -mlzcnt -mbmi2 -S "$tmp/k.c" -o "$tmp/k.s"
"$cc" -O2 -S "$tmp/k.c" -o "$tmp/k0.s"
for pair in tz:tzcnt lz:lzcnt tzq:tzcnt lzq:lzcnt dep:pdep ext:pext rl:rol; do
    f=${pair%%:*}; insn=${pair#*:}
    body=$(fn "$f" "$tmp/k.s")
    printf '%s\n' "$body" | grep -q "$insn" || bad "$f: no $insn under the ISA flags"
    printf '%s\n' "$body" | grep -Eq '\bcall|\bj[a-z]+ ' && bad "$f: call/branch under the ISA flags"
done
for f in tz lz tzq lzq rl; do
    body=$(fn "$f" "$tmp/k0.s")
    printf '%s\n' "$body" | grep -Eq 'tzcnt|lzcnt|rep[; ]*bsf' && bad "$f: BMI/LZCNT instruction on the baseline"
    printf '%s\n' "$body" | grep -q '\bcall' && bad "$f: call on the baseline"
done
[ "$fail" = 0 ] && note "PASS: instruction contracts"

[ "$fail" = 0 ] && note "check_x86_scalar_intrinsics: PASS" || { note "check_x86_scalar_intrinsics: FAIL"; exit 1; }
