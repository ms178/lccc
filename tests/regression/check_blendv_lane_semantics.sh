#!/usr/bin/env bash
# Gate: the lane-mask select (`VecBlendv*`) means the SAME thing on every x86
# target, and that meaning is the Intel sign-bit contract.
#
# The defect (engineering/SIMD-IMPLEMENTATION-AUDIT-2026-11-08.md, SA-10): the
# SSE2-baseline lowering of `_mm_blendv_ps`/`_mm_blendv_pd` used the bitwise
# `andps/andnps/orps` triple, a PER-BIT select, while the AVX path takes the
# one-instruction `vblendvps`/`vblendvpd`, a PER-LANE select keyed on each
# lane's SIGN BIT (bit 31 of a dword, bit 63 of a qword).  Masks whose lanes
# are all-ones or all-zeros hide the difference; the public intrinsic hands
# the caller's mask through verbatim, so every other mask diverged by TARGET.
#
# Three load-bearing properties, in increasing strength:
#
#   (1) INVARIANCE  each row that both ISA levels can express prints the same
#                   checksum at -msse4.1 and at -mavx.  The ISA level must not
#                   change the answer.  This is precisely what the defect
#                   violated and it needs NO reference compiler, so the gate
#                   is meaningful on any host.
#   (2) CONTRACT    each of the 16 mask-class rows equals what an independent
#                   Python model of the sign-bit reading predicts.  Guards
#                   against the opposite failure: consistently wrong on every
#                   target, which (1) alone would pass.
#   (3) CONTROL     the compare-produced row and the `_mm_blendv_epi8` row are
#                   correct BOTH before and after the fix, so they are checked
#                   for invariance and (when a reference is supplied) for
#                   equality with it -- never against the sign-bit model,
#                   which does not describe them.
#   (4) DIFFERENTIAL with --reference every LCCC row must equal the reference's
#                   row at the same ISA level.
#
# Non-vacuity is part of the contract: `--expect-defect` inverts (1) and
# requires the gate to FAIL.  Run against a pre-fix LCCC it must report red,
# otherwise the gate is decoration.
#
# The 256-bit rows exist only under AVX, so they are excluded from the
# invariance comparison (a row absent from one side is not a discrepancy);
# they are still checked against the model and the reference.
#
# Usage:
#   tests/regression/check_blendv_lane_semantics.sh [--lccc PATH] \
#       [--reference CC] [--expect-defect]

set -euo pipefail

LCCC=""
REFERENCE=""
EXPECT_DEFECT=0

while [ $# -gt 0 ]; do
    case "$1" in
        --lccc) LCCC="$2"; shift 2 ;;
        --reference) REFERENCE="$2"; shift 2 ;;
        --expect-defect) EXPECT_DEFECT=1; shift ;;
        -h|--help)
            sed -n '2,45p' "$0" | sed 's/^# \{0,1\}//'
            exit 0 ;;
        *)
            echo "check_blendv_lane_semantics: unknown argument '$1'" >&2
            exit 2 ;;
    esac
done

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SRC="$ROOT/tests/simd/blendv_lane_semantics.c"

if [ ! -f "$SRC" ]; then
    echo "FAIL: missing source $SRC" >&2
    exit 1
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

fail=0
note() { echo "  $*"; }

# Rows described by the sign-bit model: <family>_<class>.
CONTRACT_ROWS="ps128_sign_only ps128_low_bit ps128_alt ps128_mixed
pd128_sign_only pd128_low_bit pd128_alt pd128_mixed
ps256_sign_only ps256_low_bit ps256_alt ps256_mixed
pd256_sign_only pd256_low_bit pd256_alt pd256_mixed"

# Rows that must be right before AND after the fix; never model-checked.
CONTROL_ROWS="ps128_cmpmask epi8_perbit"

# ---- independent model -----------------------------------------------------

model() { # <row> -> 16 hex digits
    python3 - "$1" <<'PY'
import sys

M = 0xFFFFFFFFFFFFFFFF
PAY32 = [0x11111111, 0x22222222, 0x33333333, 0x44444444]
TRU32 = [0xF0F0F0F0, 0xF1F1F1F1, 0xF2F2F2F2, 0xF3F3F3F3]
PAY64 = [0x0123456789ABCDEF, 0xFEDCBA9876543210]
TRU64 = [0x0F0F0F0F0F0F0F0F, 0x00FF00FF00FF00FF]
PAY32_8 = [(0x11111111 * (i % 4 + 1)) & 0xFFFFFFFF for i in range(8)]
TRU32_8 = [0xF0F0F0F0 ^ ((i % 4) * 0x01010101) for i in range(8)]
PAY64_4 = [0x0123456789ABCDEF, 0xFEDCBA9876543210, 0x1111222233334444,
           0x5555666677778888]
TRU64_4 = [0x0F0F0F0F0F0F0F0F, 0x00FF00FF00FF00FF, 0x3333333333333333,
           0xCCCCCCCCCCCCCCCC]

CLASSES = ["sign_only", "low_bit", "alt", "mixed"]


def masks32(cls, n):
    return [{0: 0x80000000,
             1: 0x00000001,
             2: 0xAAAA5555 if i & 1 else 0x5555AAAA,
             3: 0x7FFFFFFF if i & 1 else 0x80000000}[cls] for i in range(n)]


def masks64(cls, n):
    return [{0: 0x8000000000000000,
             1: 0x0000000000000001,
             2: 0xAAAAAAAA55555555 if i & 1 else 0x55555555AAAAAAAA,
             3: 0x7FFFFFFFFFFFFFFF if i & 1 else 0x8000000000000000}[cls]
            for i in range(n)]


def fnv(words):
    h = 1469598103934665603
    for x, w in words:
        for b in x.to_bytes(w, "little"):
            h ^= b
            h = (h * 1099511628211) & M
    return h


def sign_select(f, t, m, bit):
    return [(t[i] if (m[i] >> bit) & 1 else f[i]) for i in range(len(m))]


name = sys.argv[1]
# The class names themselves contain underscores (`sign_only`, `low_bit`), so
# the split must be on the FIRST separator, not the last.
fam, _, cls = name.partition("_")
if fam not in ("ps128", "pd128", "ps256", "pd256") or cls not in CLASSES:
    sys.exit("model: unknown contract row %s" % name)
idx = CLASSES.index(cls)
if fam == "ps128":
    words = [(v, 4) for v in sign_select(PAY32, TRU32, masks32(idx, 4), 31)]
elif fam == "pd128":
    words = [(v, 8) for v in sign_select(PAY64, TRU64, masks64(idx, 2), 63)]
elif fam == "ps256":
    words = [(v, 4) for v in sign_select(PAY32_8, TRU32_8, masks32(idx, 8), 31)]
else:
    words = [(v, 8) for v in sign_select(PAY64_4, TRU64_4, masks64(idx, 4), 63)]
print("%016x" % fnv(words))
PY
}

have_model=1
command -v python3 >/dev/null 2>&1 || have_model=0

# ---- build and run ---------------------------------------------------------

run_rows() { "$1" | awk 'NF >= 2 {printf "%s %s\n", $1, $2}' | LC_ALL=C sort; }

build_and_run() { # <compiler> <flags> <outfile>; rows on stdout, "" on failure
    local cc="$1" flags="$2" out="$3"
    # shellcheck disable=SC2086 # flags is a deliberate word list
    if ! $cc -O2 -Wall -Wextra -Werror $flags "$SRC" -o "$out" 2>"$WORK/cc.log"; then
        cat "$WORK/cc.log" >&2
        return 1
    fi
    run_rows "$out"
}

echo "check_blendv_lane_semantics: SA-10 lane-mask select contract"

CC="${LCCC:-}"
case "$CC" in
    platform) CC="$(command -v cc || command -v gcc || true)" ;;
esac
if [ -z "$CC" ]; then
    CC="$(command -v cc || command -v gcc || true)"
fi
if [ -z "$CC" ]; then
    echo "FAIL: no C compiler (pass --lccc)" >&2
    exit 1
fi

ROWS_SSE=""
ROWS_AVX=""
if ! rows="$(build_and_run "$CC" "-msse4.1" "$WORK/lccc_sse41")"; then
    echo "FAIL: could not build the probe at -msse4.1" >&2
    exit 1
fi
ROWS_SSE="$rows"
if ! rows="$(build_and_run "$CC" "-mavx -mavx2" "$WORK/lccc_avx2")"; then
    echo "FAIL: could not build the probe at -mavx -mavx2" >&2
    exit 1
fi
ROWS_AVX="$rows"

if [ -z "$ROWS_SSE" ] || [ -z "$ROWS_AVX" ]; then
    echo "FAIL: the probe printed no rows" >&2
    exit 1
fi

# ---- (1) invariance, over the rows BOTH levels produce --------------------

inv_fail=0
inv_count=0
while read -r row hex; do
    [ -n "$row" ] || continue
    sse="$(awk -v r="$row" '$1 == r {print $2}' <<<"$ROWS_SSE")"
    # A row absent at -msse4.1 (the 256-bit family) is not a discrepancy.
    [ -n "$sse" ] || continue
    inv_count=$((inv_count + 1))
    if [ "$hex" != "$sse" ]; then
        note "INVARIANCE $row: -mavx gives $hex, -msse4.1 gives $sse"
        inv_fail=1
    fi
done <<<"$ROWS_AVX"

# The rows both ISA levels express: ps128 x4, pd128 x4, and the two
# controls.  The four 256-bit families exist only under AVX, so they are
# excluded above; a count other than 10 means the probe lost or duplicated a
# row, which would make the invariance check vacuous.
if [ "$inv_fail" -eq 0 ] && [ "$inv_count" -ne 10 ]; then
    note "compared $inv_count rows, expected 10: the probe lost or added rows"
    inv_fail=1
fi

if [ "$EXPECT_DEFECT" -eq 1 ]; then
    if [ "$inv_fail" -eq 0 ]; then
        echo "FAIL: --expect-defect but every row is already invariant" >&2
        exit 1
    fi
    echo "PASS (expected defect observed): the ISA level changes the answer"
    exit 0
fi

if [ "$inv_fail" -ne 0 ]; then
    note ""
    note "The lane-mask select must not depend on the ISA level."
    fail=1
else
    echo "  ok  $inv_count rows invariant between -msse4.1 and -mavx"
fi

# ---- (2) the sign-bit contract, from the independent model ----------------

if [ "$have_model" -eq 1 ]; then
    ctr_fail=0
    ctr_count=0
    for row in $CONTRACT_ROWS; do
        hex="$(awk -v r="$row" '$1 == r {print $2}' <<<"$ROWS_AVX")"
        if [ -z "$hex" ]; then
            note "CONTRACT $row: row missing from the probe output"
            ctr_fail=1
            continue
        fi
        want="$(model "$row")"
        ctr_count=$((ctr_count + 1))
        if [ "$hex" != "$want" ]; then
            note "CONTRACT $row: got $hex, sign-bit model predicts $want"
            ctr_fail=1
        fi
    done
    if [ "$ctr_count" -ne 16 ]; then
        note "only $ctr_count of 16 contract rows present"
        ctr_fail=1
    fi
    if [ "$ctr_fail" -ne 0 ]; then
        fail=1
    else
        echo "  ok  all 16 mask-class rows match the independent sign-bit model"
    fi
else
    echo "  note python3 unavailable: contract rows not model-checked"
fi

# ---- (3)+(4) control rows and differential against a reference ------------

if [ -n "$REFERENCE" ]; then
    if ! command -v "$REFERENCE" >/dev/null 2>&1; then
        note "note reference '$REFERENCE' not found: SKIP differential"
    else
        d_fail=0
        d_count=0
        for level in "-msse4.1" "-mavx -mavx2"; do
            ref_out="$WORK/ref$(echo "$level" | tr -dc 'a-z0-9')"
            # A reference compiler may warn; only a hard error stops the leg.
            if ! $REFERENCE -O2 $level "$SRC" -o "$ref_out" 2>"$WORK/ref.log"; then
                note "note reference cannot build '$level': SKIP that leg"
                continue
            fi
            ref_rows="$(run_rows "$ref_out")"
            ours="$ROWS_SSE"
            [ "$level" = "-mavx -mavx2" ] && ours="$ROWS_AVX"
            while read -r row hex; do
                [ -n "$row" ] || continue
                want="$(awk -v r="$row" '$1 == r {print $2}' <<<"$ref_rows")"
                [ -n "$want" ] || continue
                d_count=$((d_count + 1))
                if [ "$hex" != "$want" ]; then
                    note "DIFFERENTIAL $row at '$level': lccc $hex, reference $want"
                    d_fail=1
                fi
            done <<<"$ours"
        done
        if [ "$d_count" -eq 0 ]; then
            note "note reference produced no comparable rows: SKIP differential"
        elif [ "$d_fail" -ne 0 ]; then
            fail=1
        else
            echo "  ok  $d_count rows match $REFERENCE (controls included)"
        fi
    fi
fi

if [ "$fail" -ne 0 ]; then
    echo "FAIL: lane-mask select is not the sign-bit contract on every target" >&2
    exit 1
fi
echo "PASS: lane-mask select is target-invariant and follows the sign-bit contract"
