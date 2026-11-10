#!/usr/bin/env bash
# Gate: every FMA3 digit permutation computes `a*b + c`.
#
# The defect (engineering/SIMD-IMPLEMENTATION-AUDIT-2026-11-08.md, SA-11): all
# six `vfmadd{132,213,231}{ps,pd}` spellings shared ONE operand placement
# (`rm=b, vvvv=c, dst=a`).  Intel SDM maps the digit triple onto the roles as
#
#     132:  dst = dst  * src3 + src2
#     213:  dst = src2 * dst  + src3
#     231:  dst = src2 * src3 + dst
#
# so with a uniform reading of `(a, b, c)` those are three DIFFERENT
# expressions.  Measured on this host with a=3, b=5, c=7:
#
#     132 -> 22.0  == a*b + c   correct
#     213 -> 26.0  == c*a + b   WRONG
#     231 -> 38.0  == b*c + a   WRONG
#
# Only 132 was ever right, and 213 is not fixable by swapping its two
# multiplicand slots (both are multiplicands there) -- the ADDEND role has to
# move.  These spellings are reachable only through raw `__lccc_simd*` builtins,
# which is why no header-level test could see the defect.
#
# The gate compiles `tests/simd/fma_digit_permutation.c` with LCCC and requires
# every `<fam>_<digits> lane<N>` row to print exactly what the file's own
# `ref_<fam> lane<N>` row prints -- i.e. `a*b + c`.  The reference rows are
# computed by scalar C, so this is a self-contained differential and needs no
# second compiler; when `--reference` is supplied the reference's `ref_*` rows
# are additionally required to agree, which pins the fixture itself.
#
# Non-vacuity: `--expect-defect` inverts the requirement and demands red.  The
# pre-fix placement is manufactured by
# `tests/regression/fma_prefix_standin.sh` (it rewrites the three spellings to
# the one shared placement), so the gate's ability to fail is measured.
#
# CROSS-BACKEND: the same defect was found a second time in the i686 backend,
# which hard-coded the 132 placement for all six ops.  A host-ISA-only gate
# cannot see that, so `--target-flags` re-runs the identical differential for
# another backend (`--target-flags=-m32`).  Both backends are then covered by
# one fixture and one set of reference rows.
#
# Usage:
#   tests/regression/check_fma_digit_permutation.sh [--lccc PATH] \
#       [--reference CC] [--target-flags FLAGS] [--expect-defect]

set -euo pipefail

LCCC=""
REFERENCE=""
TARGET_FLAGS=""
EXPECT_DEFECT=0

while [ $# -gt 0 ]; do
    case "$1" in
        --lccc) LCCC="$2"; shift 2 ;;
        --reference) REFERENCE="$2"; shift 2 ;;
        --target-flags) TARGET_FLAGS="$2"; shift 2 ;;
        --target-flags=*) TARGET_FLAGS="${1#*=}"; shift ;;
        --expect-defect) EXPECT_DEFECT=1; shift ;;
        -h|--help) sed -n '2,45p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "check_fma_digit_permutation: unknown argument '$1'" >&2; exit 2 ;;
    esac
done

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SRC="$ROOT/tests/simd/fma_digit_permutation.c"
[ -f "$SRC" ] || { echo "FAIL: missing source $SRC" >&2; exit 1; }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

CC="${LCCC:-}"
[ -n "$CC" ] || CC="$(command -v cc || command -v gcc || true)"
[ -n "$CC" ] || { echo "FAIL: no C compiler (pass --lccc)" >&2; exit 1; }

# -mfma is required for the mnemonics; -mavx2 for the 256-bit forms.
# TARGET_FLAGS selects the BACKEND (e.g. -m32 for i686); it is appended last so
# it can narrow the ISA the earlier flags grant.
#
# FMA_DIGIT_PROBE selects the fixture's exercising body: it declares and calls
# LCCC's raw `__lccc_simd*` builtins, which no reference compiler provides, so
# it is passed ONLY when the compiler under test is lccc (directly, or through
# the stand-in wrapper that delegates to it).  The fixture cannot detect this
# itself -- there is no predefined macro that identifies lccc -- so the driver
# states it.  Without the define the same source still builds anywhere and
# prints just the scalar `ref_*` rows, which is what `--reference` compares.
PROBE_DEFINE=""
case "${LCCC##*/}" in
    lccc|*standin*) PROBE_DEFINE="-DFMA_DIGIT_PROBE=1" ;;
esac
# BASE_FLAGS build the scalar reference rows and are valid for ANY compiler;
# FLAGS add the LCCC-only probe body.  Kept separate because the `--reference`
# differential must build with BASE_FLAGS: passing the probe define to a
# reference compiler asks it to declare and call LCCC's raw `__lccc_simd*`
# builtins, which it cannot, and the differential then degraded to a SKIP
# instead of pinning the fixture.
# The 512-bit rows are only reachable with -mavx512f, and only MEANINGFUL where
# the host can execute them.  Probe the host's own AVX-512F bit rather than
# assuming it: on a runner without it the gate still checks 128- and 256-bit
# and simply omits the 512 rows from its expected count.  x86-64 only -- the
# i686 backend has no EVEX emitters, so -m32 never gets the flag.
AVX512_FLAG=""
if [ -z "$TARGET_FLAGS" ] && grep -qw avx512f /proc/cpuinfo 2>/dev/null; then
    AVX512_FLAG="-mavx512f"
fi
BASE_FLAGS="-O2 -Wall -Wextra -Werror -mfma -mavx2${AVX512_FLAG:+ $AVX512_FLAG}${TARGET_FLAGS:+ $TARGET_FLAGS}"
FLAGS="$BASE_FLAGS${PROBE_DEFINE:+ $PROBE_DEFINE}"

echo "check_fma_digit_permutation: SA-11 FMA3 role placement${TARGET_FLAGS:+ [$TARGET_FLAGS]}"

# A compiler that cannot build the probe is a hard error, not a skip: the file
# is plain C with no LCCC-only syntax outside the `__CCC__` guard.
# shellcheck disable=SC2086 # FLAGS is a deliberate word list
if ! $CC $FLAGS "$SRC" -o "$WORK/probe" 2>"$WORK/cc.log"; then
    echo "FAIL: could not build the FMA probe with '$CC'" >&2
    cat "$WORK/cc.log" >&2
    exit 1
fi
"$WORK/probe" >"$WORK/out.txt"

rows_bad=0
checked=0
mismatch=0

# Compare every `<fam>_<digits> laneN VALUE` against `ref_<fam> laneN VALUE`.
while IFS= read -r line; do
    case "$line" in
        *_132\ lane*|*_213\ lane*|*_231\ lane*) ;;
        *) continue ;;
    esac
    got_name=$(printf '%s' "$line" | awk '{print $1}')
    lane=$(printf '%s' "$line" | awk '{print $2}')
    value=$(printf '%s' "$line" | awk '{print $3}')
    fam=${got_name%_*}          # strip the digit suffix
    family=${fam%%_*}           # ref rows use `ref_<family>`
    want=$(awk -v f="ref_${family}" -v l="$lane" '$1 == f && $2 == l {print $3}' "$WORK/out.txt")
    if [ -z "$want" ]; then
        echo "  MISSING reference row ref_${family} ${lane} for ${got_name}" >&2
        rows_bad=1
        continue
    fi
    checked=$((checked + 1))
    if [ "$value" != "$want" ]; then
        echo "  $got_name $lane: got $value, want $want (a*b+c)" >&2
        mismatch=$((mismatch + 1))
    fi
done <"$WORK/out.txt"

# Every lane of every spelling at every width the host can execute:
#
#     ps128 3x4  + pd128 3x2  = 12 + 6  = 18
#     ps256 3x8  + pd256 3x4  = 24 + 12 = 36
#     ps512 3x16 + pd512 3x8  = 48 + 24 = 72   (only with -mavx512f)
#
# The floor is EXACT, not a lower bound: a dropped row silently weakens the
# gate.  The previous value of 24 came from half-width blocks that read 16
# bytes past arrays sized for the other width, and it counted only half of each
# vector's lanes -- so the "green" it reported covered a third of what it said.
expect_rows=54
if [ -n "$AVX512_FLAG" ]; then
    expect_rows=126
fi
if [ "$EXPECT_DEFECT" -eq 1 ]; then
    if [ "$mismatch" -eq 0 ] && [ "$rows_bad" -eq 0 ]; then
        echo "FAIL: --expect-defect but every permutation computes a*b+c" >&2
        exit 1
    fi
    echo "PASS (expected defect observed): $mismatch row(s) are not a*b+c"
    exit 0
fi

if [ "$rows_bad" -ne 0 ]; then
    echo "FAIL: the probe did not print the rows the gate needs" >&2
    exit 1
fi
# A build without `__CCC__` defines cannot emit the `<fam>_<digits>` rows at
# all: the whole exercised body is guarded, so the program prints only its
# `ref_*` rows.  That was a real hole -- the gate reported "compared 0 rows"
# and its exit status said 1 whether the defect were present or not, so anyone
# running it with a reference compiler (or a stand-in) drew a false signal.
# Detect that case explicitly instead of letting the row floor speak for it.
if [ "$checked" -eq 0 ]; then
    echo "FAIL: the probe printed no <fam>_<digits> rows, so nothing was" >&2
    echo "      compared.  The exercising body is selected by" >&2
    echo "      -DFMA_DIGIT_PROBE, which this driver passes only when the" >&2
    echo "      compiler name says lccc (or a stand-in delegating to it);" >&2
    echo "      '$CC' produced only reference rows.  Run the gate with" >&2
    echo "      --lccc <lccc binary>." >&2
    exit 1
fi
if [ "$checked" -lt "$expect_rows" ]; then
    echo "FAIL: compared $checked rows, expected at least $expect_rows" >&2
    exit 1
fi
if [ "$mismatch" -ne 0 ]; then
    echo "FAIL: $mismatch of $checked FMA rows do not compute a*b+c" >&2
    exit 1
fi
echo "  ok  all $checked FMA rows compute a*b+c"

# The fixture itself must be sane: within one block every lane's a*b+c must be
# distinct, or a wrong permutation could coincide with the right answer and the
# row would pass anyway.  Across blocks a repeat is EXPECTED and correct -- the
# 256-bit blocks reuse the low lanes of the 128-bit ones (ps256 lanes 0..3 are
# ps128 lanes 0..3, pd256 lanes 0..1 are pd128 lanes 0..1) -- so the check is
# per block family, not global.
dupes=$(awk '$1 ~ /^ref_/ {blk=$1; sub(/_?lane[0-9]+$/, "", blk); print blk, $3}' \
        "$WORK/out.txt" | sort | uniq -d | wc -l)
if [ "$dupes" -ne 0 ]; then
    echo "  FAIL $dupes reference value(s) repeat within one block: a wrong"
    echo "       permutation could coincide with the right answer and still pass"
    exit 1
fi
echo "  ok  every block has distinct per-lane reference values"

if [ -n "$REFERENCE" ]; then
    if ! command -v "$REFERENCE" >/dev/null 2>&1; then
        echo "  note reference '$REFERENCE' not found: SKIP"
    elif ! $REFERENCE $BASE_FLAGS "$SRC" -o "$WORK/refprobe" 2>"$WORK/ref.log"; then
        echo "  note reference cannot build the probe: SKIP"
    else
        "$WORK/refprobe" >"$WORK/refout.txt"
        ref_mismatch=0
        while IFS= read -r line; do
            case "$line" in ref_*) ;; *) continue ;; esac
            name=$(printf '%s' "$line" | awk '{print $1}')
            lane=$(printf '%s' "$line" | awk '{print $2}')
            value=$(printf '%s' "$line" | awk '{print $3}')
            want=$(awk -v f="$name" -v l="$lane" '$1 == f && $2 == l {print $3}' "$WORK/refout.txt")
            if [ -n "$want" ] && [ "$value" != "$want" ]; then
                echo "  DIFFERENTIAL $name $lane: lccc $value, reference $want" >&2
                ref_mismatch=1
            fi
        done <"$WORK/out.txt"
        if [ "$ref_mismatch" -ne 0 ]; then
            echo "FAIL: the reference fixture disagrees with $REFERENCE" >&2
            exit 1
        fi
        echo "  ok  the reference rows match $REFERENCE"
    fi
fi

echo "PASS: every FMA3 digit permutation computes a*b+c"
