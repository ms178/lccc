#!/usr/bin/env bash
# Emission + semantics gate for the two-wide (SSE2) matmul arm.
#
# WHY THIS EXISTS
# ---------------
# The matmul arm's only two-wide lowering used to be the fused one
# (`FmaF64x2Hoisted` -> `vfmadd231pd` / `fmla`), so the contraction
# precondition `fp_contract == fast && -mfma` also decided whether the loop was
# vectorized AT ALL.  When plain x86-64 (SSE2, no FMA3) became the default
# codegen baseline, every matmul stencil on a default `-O2` build silently went
# scalar: the benchmark's N=1024 kernel ran 601 ms where the same source at
# `-march=x86-64-v3` ran 278 ms and GCC 16.2 ran 361 ms.  Nothing failed -- the
# transform simply declined, with a diagnostic only under
# `LCCC_DEBUG_VECTORIZE`.
#
# A refused contraction is not a refusal to vectorize.  This gate pins the
# matrix of that contract, by EMISSION and by EXECUTED RESULT:
#
#   default (plain x86-64)        packed `mulpd`/`addpd`, no FMA, no ymm
#   -mno-fma                      same (a denied unit is not a denied vector)
#   -ffp-contract=off             same (a withdrawn licence keeps the packing)
#   -march=x86-64-v3              fused `vfmadd231pd` on 256-bit vectors
#   -march=x86-64-v3 -mno-fma     `vmulpd` + `vaddpd`, still 256-bit, no FMA
#   -march=x86-64-v3 -ffp-contract=off
#                                 `vmulpd` + `vaddpd`, still 256-bit, no FMA
#   -mno-sse                      REFUSES the FP translation unit (x87 is not
#                                 implemented) -- it must never fall back to
#                                 emitting SSE for a kernel TU
#
# Every compiling configuration is also RUN and must print the same hash as the
# scalar reference, except where the rounding genuinely differs: the two
# fused configurations must print a DIFFERENT hash from the two-rounding ones,
# which is what proves the rounding choice is observable in this data (and
# therefore that the in-source `volatile` reference has teeth).
set -u -o pipefail

repo=$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)
ccc=${CCC:-$repo/target/fastbuild/lccc}
src=$repo/tests/regression/vectorize_matmul_sse2_baseline.c
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

fail=0
count() { # count <regex> <file>
    grep -cE "$1" "$2" 2>/dev/null || true
}
check_eq() { # check_eq <desc> <actual> <expected>
    if [ "$2" != "$3" ]; then
        echo "FAIL: $1 -- got '$2', want '$3'" >&2
        fail=1
    fi
}
check_gt0() { # check_gt0 <desc> <actual>
    if [ "$2" -eq 0 ] 2>/dev/null; then
        echo "FAIL: $1 -- expected a non-zero count, got 0" >&2
        fail=1
    fi
}

# build <label> [flags...]: asm + binary, no diagnostics expected.
build() {
    local label=$1
    shift
    if ! "$ccc" -O2 "$@" -S "$src" -o "$tmp/$label.s" 2> "$tmp/$label.err"; then
        echo "FAIL: $label -- compile (asm) failed" >&2
        sed -n '1,5p' "$tmp/$label.err" >&2
        fail=1
        return 1
    fi
    if [ -s "$tmp/$label.err" ]; then
        echo "FAIL: $label -- unexpected diagnostics:" >&2
        sed -n '1,5p' "$tmp/$label.err" >&2
        fail=1
    fi
    if ! "$ccc" -O2 "$@" "$src" -o "$tmp/$label.bin" 2>> "$tmp/$label.err"; then
        echo "FAIL: $label -- link failed" >&2
        fail=1
        return 1
    fi
    if [ -s "$tmp/$label.err" ]; then
        echo "FAIL: $label -- unexpected diagnostics (link):" >&2
        sed -n '1,5p' "$tmp/$label.err" >&2
        fail=1
    fi
    if ! "$tmp/$label.bin" > "$tmp/$label.out" 2>&1; then
        echo "FAIL: $label -- execution failed:" >&2
        sed -n '1,5p' "$tmp/$label.out" >&2
        fail=1
        return 1
    fi
    if ! grep -qx 'OK' "$tmp/$label.out"; then
        echo "FAIL: $label -- self-check did not report OK:" >&2
        sed -n '1,5p' "$tmp/$label.out" >&2
        fail=1
    fi
    return 0
}

hash_of() { head -1 "$tmp/$1.out"; }

# ── packed, unfused, 128-bit ─────────────────────────────────────────────────
# The default baseline is the configuration the regression shipped in.
if build baseline; then
    check_gt0 "baseline: packed mul" "$(count '\bmulpd\b' "$tmp/baseline.s")"
    check_gt0 "baseline: packed add" "$(count '\baddpd\b' "$tmp/baseline.s")"
    check_eq "baseline: no FMA (no FMA3 on x86-64 v1)" \
        "$(count '\bvfmadd[0-9]*p[sd]\b|\bvfmsub[0-9]*p[sd]\b|\bvmulpd\b|\bvaddpd\b' \
           "$tmp/baseline.s")" 0
    check_eq "baseline: no 256-bit vector register" \
        "$(count '%ymm' "$tmp/baseline.s")" 0
fi

if build no_fma -mno-fma; then
    check_gt0 "mno-fma: packed mul" "$(count '\bmulpd\b' "$tmp/no_fma.s")"
    check_eq "mno-fma: no packed FMA" \
        "$(count '\bvfmadd[0-9]*p[sd]\b' "$tmp/no_fma.s")" 0
    check_eq "mno-fma: no 256-bit vector register" \
        "$(count '%ymm' "$tmp/no_fma.s")" 0
fi

if build contract_off -ffp-contract=off; then
    check_gt0 "ffp-contract=off: packed mul" "$(count '\bmulpd\b' "$tmp/contract_off.s")"
    check_gt0 "ffp-contract=off: packed add" "$(count '\baddpd\b' "$tmp/contract_off.s")"
    check_eq "ffp-contract=off: no packed FMA" \
        "$(count '\bvfmadd[0-9]*p[sd]\b' "$tmp/contract_off.s")" 0
    check_eq "ffp-contract=off: no 256-bit vector register" \
        "$(count '%ymm' "$tmp/contract_off.s")" 0
fi

# ── 256-bit ─────────────────────────────────────────────────────────────────
# The fused path is unchanged by the baseline fix; it must stay exactly what it
# was (`vfmadd231pd` on ymm), and it is the only configuration whose numerical
# result may differ from the two-rounding ones.
if build v3 -march=x86-64-v3; then
    check_gt0 "x86-64-v3: fused FMA" "$(count '\bvfmadd231pd\b' "$tmp/v3.s")"
    check_gt0 "x86-64-v3: 256-bit vectors" "$(count '%ymm' "$tmp/v3.s")"
fi

if build v3_no_fma -march=x86-64-v3 -mno-fma; then
    check_gt0 "x86-64-v3 -mno-fma: packed mul" "$(count '\bvmulpd\b' "$tmp/v3_no_fma.s")"
    check_gt0 "x86-64-v3 -mno-fma: packed add" "$(count '\bvaddpd\b' "$tmp/v3_no_fma.s")"
    check_eq "x86-64-v3 -mno-fma: no packed FMA" \
        "$(count '\bvfmadd[0-9]*p[sd]\b' "$tmp/v3_no_fma.s")" 0
fi

if build v3_contract_off -march=x86-64-v3 -ffp-contract=off; then
    check_gt0 "x86-64-v3 ffp-contract=off: packed mul" \
        "$(count '\bvmulpd\b' "$tmp/v3_contract_off.s")"
    check_eq "x86-64-v3 ffp-contract=off: no packed FMA" \
        "$(count '\bvfmadd[0-9]*p[sd]\b' "$tmp/v3_contract_off.s")" 0
fi

# Force the two-wide IR path on FMA-capable hardware. This used to stay scalar
# when contraction was denied, and the new non-fused op was never constructed.
# Both AVX1+FMA and AVX2 exercise the distinction between encoding and semantics.
for isa in avx_fma v3; do
    if [ "$isa" = avx_fma ]; then
        flags=(-mavx -mfma)
    else
        flags=(-march=x86-64-v3)
    fi
    if LCCC_FORCE_SSE2=1 build "${isa}_two_wide_off" "${flags[@]}" -ffp-contract=off; then
        check_gt0 "$isa two-wide off: packed mul" \
            "$(count '\bvmulpd\b' "$tmp/${isa}_two_wide_off.s")"
        check_gt0 "$isa two-wide off: packed add" \
            "$(count '\bvaddpd\b' "$tmp/${isa}_two_wide_off.s")"
        check_eq "$isa two-wide off: no FMA" \
            "$(count '\bvfmadd[0-9]*p[sd]\b' "$tmp/${isa}_two_wide_off.s")" 0
        check_eq "$isa two-wide off: same separately-rounded result" \
            "$(hash_of "${isa}_two_wide_off")" "$(hash_of baseline)"
    fi
done

# ── the one direction that must not vectorize ──────────────────────────────
# `-mno-sse` (the kernel's flag set) removes the xmm file, so no packed form is
# legal.  FP has no x87 lowering here, so the TU must be refused outright --
# what must NEVER happen is a silent SSE-emitted object.
if "$ccc" -O2 -mno-sse -S "$src" -o "$tmp/mno_sse.s" 2> "$tmp/mno_sse.err"; then
    if [ -s "$tmp/mno_sse.s" ]; then
        if [ "$(count '\b(mulpd|addpd|movupd|movsd)\b' "$tmp/mno_sse.s")" != 0 ]; then
            echo "FAIL: -mno-sse emitted SSE instructions" >&2
            fail=1
        fi
    fi
else
    if ! grep -qE 'requires SSE|regs-only' "$tmp/mno_sse.err"; then
        echo "FAIL: -mno-sse refused the FP unit without the documented diagnostic:" >&2
        sed -n '1,5p' "$tmp/mno_sse.err" >&2
        fail=1
    fi
fi

# ── the rounding choice is observable ──────────────────────────────────────
if [ -s "$tmp/baseline.out" ] && [ -s "$tmp/v3.out" ]; then
    if [ "$(hash_of baseline)" = "$(hash_of v3)" ]; then
        echo "FAIL: fused and two-rounding builds produced the same hash;" \
             "the reference loop no longer distinguishes them" >&2
        fail=1
    fi
fi
for cfg in no_fma contract_off v3_no_fma v3_contract_off; do
    if [ -s "$tmp/$cfg.out" ] && [ -s "$tmp/baseline.out" ]; then
        if [ "$(hash_of $cfg)" != "$(hash_of baseline)" ]; then
            echo "FAIL: $cfg must round twice, like the baseline" \
                 "($(hash_of $cfg) != $(hash_of baseline))" >&2
            fail=1
        fi
    fi
done

if [ "$fail" -eq 0 ]; then
    echo "matmul SSE2 baseline gates: PASS"
fi
exit "$fail"
