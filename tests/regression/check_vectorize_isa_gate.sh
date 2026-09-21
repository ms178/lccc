#!/usr/bin/env bash
# Emission + semantics gate for the x86 SIMD ISA contract of the middle end.
#
#   default / -march=x86-64          -> AVX2 vectorization (project baseline
#                                       is x86-64-v3; measured benchmark data
#                                       depends on this, so it must not
#                                       silently regress)
#   -mno-avx / -mno-avx2             -> 128-bit SSE2 vectorization: downgraded,
#                                       NOT disabled, and zero ymm
#   -mno-sse -mno-mmx -mno-sse2
#   -mno-avx (the kernel's flag set) -> no SIMD register reference at all
#   fma/fmaf                         -> vfmadd* by default, never under
#                                       -mno-sse / -mno-avx / -mno-fma
#
# Every configuration is also *executed*: a gate that merely stopped
# vectorizing would pass the emission checks while silently degrading or
# breaking the loop, so the semantics are pinned too.
set -euo pipefail

repo=$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)
ccc=${CCC:-$repo/target/fastbuild/lccc}
src=$repo/tests/regression/vectorize_isa_gate.c
fmasrc=$repo/tests/regression/fma_isa_gate.c
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# The exact SIMD-related subset of the kernel's KBUILD_CFLAGS.
KERNEL_ISA=(-mno-sse -mno-mmx -mno-sse2 -mno-3dnow -mno-avx -mno-sse4a)

fail=0
check_eq() { # check_eq <desc> <actual> <expected>
    if [[ "$2" != "$3" ]]; then
        echo "FAIL: $1 -- got '$2', want '$3'" >&2
        fail=1
    fi
}
check_gt0() { # check_gt0 <desc> <actual>
    if [[ "$2" -eq 0 ]]; then
        echo "FAIL: $1 -- expected a non-zero count, got 0" >&2
        fail=1
    fi
}
count_simd() { grep -cE '\b[xyz]mm[0-9]+\b' "$1" || true; }
count_ymm() { grep -cE '\bymm[0-9]+\b' "$1" || true; }
count_vfmadd() { grep -cE '\bvfmadd[0-9]*p?[sd]\b|\bvfmadd' "$1" || true; }
count_bitcount_zero_guards() { grep -cE '^\.Lc[lt]z_nz_' "$1" || true; }
count_bsf() { grep -cE '\bbsf[lq]\b' "$1" || true; }
count_bsr() { grep -cE '\bbsr[lq]\b' "$1" || true; }

run_cfg() { # run_cfg <label> <flags...> ; asserts emission + execution
    local label=$1; shift
    "$ccc" -O2 "$@" -S "$src" -o "$tmp/$label.s"
    "$ccc" -O2 "$@" "$src" -o "$tmp/$label.bin"
    local out
    out=$("$tmp/$label.bin")
    check_eq "$label semantics" "$out" "fail=0"
}

# ---- 1. default: AVX2 baseline must survive -------------------------------
run_cfg default
check_gt0 "default AVX2 vectorization (ymm)" "$(count_ymm $tmp/default.s)"
# __builtin_clz/ctz have a nonzero source precondition. On baseline x86 the
# optimal lowering is branchless BSR/BSF; a .Lclz_nz/.Lctz_nz label proves the
# backend accidentally restored its internal defined-zero semantics.
check_eq "baseline nonzero clz/ctz zero-fixup guards" \
    "$(count_bitcount_zero_guards $tmp/default.s)" 0
check_gt0 "baseline nonzero ctz uses BSF" "$(count_bsf $tmp/default.s)"
check_gt0 "baseline nonzero clz uses BSR" "$(count_bsr $tmp/default.s)"
check_eq "baseline direct CTZ has no dead rdi-to-rax preload" \
    "$(grep -c 'movq %rdi, %rax' "$tmp/default.s" || true)" 0

run_cfg march-v3 -march=x86-64-v3
check_gt0 "-march=x86-64-v3 AVX2 vectorization (ymm)" "$(count_ymm $tmp/march-v3.s)"

# ---- 2. -mno-avx: downgrade to SSE2, keep vectorizing ---------------------
for flag in -mno-avx -mno-avx2 -mno-sse4.1; do
    lbl=${flag#-m}; lbl=n${lbl#no-}
    run_cfg "$lbl" "$flag"
    check_eq "$flag ymm emission" "$(count_ymm $tmp/$lbl.s)" 0
    check_gt0 "$flag still vectorizes (xmm)" "$(count_simd $tmp/$lbl.s)"
done

# ---- 3. kernel flag set: no SIMD register at all --------------------------
run_cfg kernel-isa "${KERNEL_ISA[@]}"
check_eq "kernel ISA flags: SIMD register refs" "$(count_simd $tmp/kernel-isa.s)" 0

# -O3 runs the same pipeline with unrolling on top.
"$ccc" -O3 "${KERNEL_ISA[@]}" -S "$src" -o "$tmp/kernel-isa-o3.s"
check_eq "kernel ISA flags at -O3: SIMD register refs" \
    "$(count_simd $tmp/kernel-isa-o3.s)" 0

# The Phase-2b memory-form ARX vectorizer (`vec_arx`) runs BEFORE the main
# vectorizer's ISA gate yet emits XMM-shaped intrinsics on the u32[16]
# ChaCha slot domain.  Before its own gate it turned a kernel TU into a
# hard backend rejection ("offending instruction: `movdqu (%rax), %xmm2'").
# Both the array spelling (benchmark chacha20_block.c) and the phi spelling
# (regression vec_arx_scalar_spelling.c) must stay scalar and compile.
for arx in "$repo/tests/regression/vec_arx_scalar_spelling.c" \
           "$repo/tests/benchmark/programs/chacha20_block.c"; do
    name=$(basename "$arx" .c)
    "$ccc" -O2 "${KERNEL_ISA[@]}" -S "$arx" -o "$tmp/arx-$name.s"
    check_eq "kernel ISA flags: ARX $name xmm refs" \
        "$(count_simd "$tmp/arx-$name.s")" 0
done
"$ccc" -O3 "${KERNEL_ISA[@]}" -S "$repo/tests/benchmark/programs/chacha20_block.c" \
    -o "$tmp/arx-chacha-o3.s"
check_eq "kernel ISA flags at -O3: ARX chacha20_block xmm refs" \
    "$(count_simd "$tmp/arx-chacha-o3.s")" 0

# ---- 4. FMA3 gate ----------------------------------------------------------
"$ccc" -O2 -S "$fmasrc" -o "$tmp/fma-default.s"
check_gt0 "default fmaf folds to vfmadd" "$(count_vfmadd $tmp/fma-default.s)"
for flag in -mno-fma -mno-avx -march=x86-64-v2; do
    "$ccc" -O2 "$flag" -S "$fmasrc" -o "$tmp/fma.s"
    check_eq "$flag: vfmadd emission" "$(count_vfmadd $tmp/fma.s)" 0
done
# ---- 4b. -mno-avx2: the AVX1+FMA target class -------------------------------
# AVX2 is the 256-bit integer class, a strict SUBSET of the VEX encoding:
# `-mno-avx2` must remove ymm code but keep the VEX.128 world — including
# every scalar FMA family (plain AND signed). The pre-fix ISA ceiling killed
# `avx` on this flag, so `fma(-a,b,-c)` at `-mno-avx2` was two `xorpd` and a
# libm call where GCC emits one `vfnmsub132sd`; the plain family declined
# the same way. Both spellings of the target (default baseline minus AVX2,
# and an explicit v3 profile minus AVX2) are pinned, plus the ymm denial.
count_fma_family() {
    grep -cE '\bv(fm|fnm)(add|sub)[0-9]*p?s[sd]?\b|\bv(fm|fnm)(add|sub)' "$1" || true
}
for flag in "-mno-avx2" "-march=x86-64-v3 -mno-avx2"; do
    lbl=$(echo "$flag" | tr -d ' =-')
    "$ccc" -O2 $flag -S "$fmasrc" -o "$tmp/fma-$lbl.s"
    check_gt0 "$flag: scalar FMA families stay inline" \
        "$(count_fma_family $tmp/fma-$lbl.s)"
    check_gt0 "$flag: the negated families (vfnmsub et al) stay inline" \
        "$(grep -cE '\bvfnm(add|sub)[0-9]*p?s[dd]?\b' "$tmp/fma-$lbl.s" || true)"
    check_eq "$flag: no libm fma call" "$(grep -c 'fma@PLT' "$tmp/fma-$lbl.s" || true)" 0
    check_eq "$flag: no 256-bit code" "$(count_ymm "$tmp/fma-$lbl.s")" 0
    # The multi-use negation shape: both fma sites fold, the xorpd bracket
    # dies with them. (lccc emits no .size directives; the function range
    # ends at the next column-0 label, which is `main:` in this corpus.)
    # NOTE the regex: `\bxorpd\b` can never match the VEX spelling `vxorpd`
    # (no word boundary between v and x), and every 4b target is VEX, so
    # the pin must be `v?xorpd` to see anything at all.
    check_eq "$flag: shared-negation shape leaves no sign-mask xorpd" \
        "$(sed -n '/^shared_neg:/,/^main:/p' "$tmp/fma-$lbl.s" \
          | grep -cE '\bv?xorpd\b' || true)" 0
    check_gt0 "$flag: shared-negation shape folds (two vfnmsub present)" \
        "$(sed -n '/^shared_neg:/,/^main:/p' "$tmp/fma-$lbl.s" \
          | grep -cE '\bvfnmsub[0-9]*p?sd\b' || true)"
    check_eq "$flag: shared-negation shape is exactly two vfnmsub" \
        "$(sed -n '/^shared_neg:/,/^main:/p' "$tmp/fma-$lbl.s" \
          | grep -cE '\bvfnmsub[0-9]*p?sd\b' || true)" 2
    # The chain shapes (the fixpoint correction). chain_live: the fma reads
    # the INNER negation while the OUTER survives for the add -- both links
    # materialise (exactly two sign masks) and NO latitude is taken (the
    # family stays plain: zero negated families). The source-poisoned rule
    # deleted the inner link under the outer's surviving read -- an ICE.
    check_eq "$flag: live-outer chain keeps exactly two sign masks" \
        "$(sed -n '/^chain_live:/,/^chain_pin:/p' "$tmp/fma-$lbl.s" \
          | grep -cE '\bv?xorpd\b' || true)" 2
    check_eq "$flag: live-outer chain takes no sign latitude (no vfnm*)" \
        "$(sed -n '/^chain_live:/,/^chain_pin:/p' "$tmp/fma-$lbl.s" \
          | grep -cE '\bvfnm(add|sub)[0-9]*p?sd\b' || true)" 0
    # chain_pin: the INNER negation is pinned by the add, the site reads
    # the OUTER -- which peels into the family reading the materialised
    # inner: ONE sign mask and one vfnmadd (GCC's exact shape; the
    # source-poisoned rule kept both masks).
    check_eq "$flag: pinned-inner chain keeps exactly one sign mask" \
        "$(sed -n '/^chain_pin:/,/^shared_neg:/p' "$tmp/fma-$lbl.s" \
          | grep -cE '\bv?xorpd\b' || true)" 1
    check_eq "$flag: pinned-inner chain folds to one vfnmadd" \
        "$(sed -n '/^chain_pin:/,/^shared_neg:/p' "$tmp/fma-$lbl.s" \
          | grep -cE '\bvfnmadd[0-9]*p?sd\b' || true)" 1
done
# The kernel's no-SSE contract also rejects live scalar FP: x86-64 LCCC has
# no x87 lowering, and silently accepting this TU would be worse than a clear
# diagnostic.  Keep the FMA gate assertion separate from that diagnostic.
if "$ccc" -O2 "${KERNEL_ISA[@]}" -S "$fmasrc" -o "$tmp/fma.s" \
    >"$tmp/kernel-fma.out" 2>"$tmp/kernel-fma.err"; then
    echo "FAIL: kernel ISA flags accepted a live scalar-FP FMA TU" >&2
    fail=1
elif ! grep -q "floating-point operation requires SSE" "$tmp/kernel-fma.err"; then
    echo "FAIL: kernel ISA FMA diagnostic changed or disappeared" >&2
    cat "$tmp/kernel-fma.err" >&2
    fail=1
fi

# ---- 5. `-msse` after `-mno-sse` re-enables (kernel CC_FLAGS_FPU) ----------
# arch/x86/Makefile appends -msse to CC_FLAGS_FPU for FPU-using TUs, after the
# global -mno-sse; GCC's last-explicit-ISA-flag-wins must be honoured.
"$ccc" -O2 "${KERNEL_ISA[@]}" -msse -msse2 -S "$src" -o "$tmp/resse.s"
check_gt0 "-mno-sse ... -msse re-enables vectorization" "$(count_simd $tmp/resse.s)"


# ---- 6. ISA-gate trace contract --------------------------------------------
# The gate is answered BEFORE the natural-loop analysis, so a TU compiled with
# the kernel's -mno-sse does not build a loop forest that the gate then throws
# away (measured on one real TU, mm/page_alloc.c of 6.18.52: 814 entries into
# vectorize_with_analysis_mode, 814 refusals).  Hoisting a gate above the work
# it guards is exactly the kind of change that silently drops a diagnostic, so
# the trace is pinned here:
#
#   * LCCC_DEBUG_VECTORIZE prints the header line — which reports the loop
#     count — and then the refusal, per function, in that order and paired by
#     function name;
#   * the loop count is still the REAL one, i.e. under trace the analysis runs
#     exactly as it did when the gate came last;
#   * LCCC_WHY_NOT_VECTORIZE alone prints only the refusals, and agrees with
#     the debug run on how many functions the gate refused;
#   * neither variable prints anything at all.
hdr_re='^\[VEC\] Function: [A-Za-z_][A-Za-z0-9_]*, blocks: [0-9]+, loops: [0-9]+$'
ref_re='^\[VEC\] Function [A-Za-z_][A-Za-z0-9_]*: not vectorized: x86 SIMD disabled by ISA flags \(-mno-sse/-mno-sse2/-mgeneral-regs-only\)$'

LCCC_DEBUG_VECTORIZE=1 "$ccc" -O2 "${KERNEL_ISA[@]}" -S "$src" -o "$tmp/tr-dbg.s"   2>"$tmp/tr-dbg.err"
LCCC_WHY_NOT_VECTORIZE=1 "$ccc" -O2 "${KERNEL_ISA[@]}" -S "$src" -o "$tmp/tr-why.s" 2>"$tmp/tr-why.err"
"$ccc" -O2 "${KERNEL_ISA[@]}" -S "$src" -o "$tmp/tr-quiet.s" 2>"$tmp/tr-quiet.err"

n_hdr=$(grep -cE "$hdr_re" "$tmp/tr-dbg.err" || true)
n_ref=$(grep -cE "$ref_re" "$tmp/tr-dbg.err" || true)
n_nonblank=$(grep -cvE '^[[:space:]]*$' "$tmp/tr-dbg.err" || true)
check_gt0 "trace: header lines under LCCC_DEBUG_VECTORIZE + kernel ISA flags" "${n_hdr:-0}"
check_eq  "trace: one refusal per header line" "$n_hdr" "$n_ref"
check_eq  "trace: the gate prints nothing besides the paired lines" \
          "$n_nonblank" "$((n_hdr + n_ref))"

# Pairing and ORDER: every header must be immediately followed by the refusal
# for the same function, with no stray line in between or after.
pair_err=$(awk '
    /^\[VEC\] Function: / {
        name = $3; sub(/,$/, "", name); pending = name; next
    }
    /: not vectorized: x86 SIMD disabled/ {
        name = $3; sub(/:$/, "", name)
        if (pending == "")      printf "refusal without a header line: %s\n", name
        else if (pending != name) printf "header/refusal name mismatch: %s != %s\n", pending, name
        pending = ""; next
    }
    { printf "stray trace line: %s\n", $0 }
    END { if (pending != "") printf "header without a refusal: %s\n", pending }
' "$tmp/tr-dbg.err")
if [[ -n $pair_err ]]; then
    echo "FAIL: ISA-gate trace pairing/order" >&2
    printf '%s\n' "$pair_err" >&2
    fail=1
fi

# The loop count must be the analysed one, not a placeholder: if the hoist ever
# skipped find_natural_loops under trace, every count would collapse to 0.
max_loops=$(grep -oE 'loops: [0-9]+' "$tmp/tr-dbg.err" | awk '{print $2}' | sort -n | tail -1)
check_gt0 "trace: the natural-loop analysis still runs under trace" "${max_loops:-0}"

check_eq "trace: LCCC_WHY_NOT_VECTORIZE alone prints no header lines" \
         "$(grep -cE "$hdr_re" "$tmp/tr-why.err" || true)" 0
check_eq "trace: both variables agree on the refusal count" \
         "$(grep -cE "$ref_re" "$tmp/tr-why.err" || true)" "$n_ref"
check_eq "trace: no variable, no output" \
         "$(wc -c < "$tmp/tr-quiet.err" | tr -d '[:space:]')" 0


# ---- 6. ISA-gate trace contract --------------------------------------------
# The gate is answered BEFORE the natural-loop analysis, so a TU compiled with
# the kernel's -mno-sse does not build a loop forest that the gate then throws
# away (measured on one real TU, mm/page_alloc.c of 6.18.52: 814 entries into
# vectorize_with_analysis_mode, 814 refusals).  Hoisting a gate above the work
# it guards is exactly the kind of change that silently drops a diagnostic, so
# the trace is pinned here:
#
#   * LCCC_DEBUG_VECTORIZE prints the header line — which reports the loop
#     count — and then the refusal, per function, in that order and paired by
#     function name;
#   * the loop count is still the REAL one, i.e. under trace the analysis runs
#     exactly as it did when the gate came last;
#   * LCCC_WHY_NOT_VECTORIZE alone prints only the refusals, and agrees with
#     the debug run on how many functions the gate refused;
#   * neither variable prints anything at all.
hdr_re='^\[VEC\] Function: [A-Za-z_][A-Za-z0-9_]*, blocks: [0-9]+, loops: [0-9]+$'
ref_re='^\[VEC\] Function [A-Za-z_][A-Za-z0-9_]*: not vectorized: x86 SIMD disabled by ISA flags \(-mno-sse/-mno-sse2/-mgeneral-regs-only\)$'

LCCC_DEBUG_VECTORIZE=1 "$ccc" -O2 "${KERNEL_ISA[@]}" -S "$src" -o "$tmp/tr-dbg.s"   2>"$tmp/tr-dbg.err"
LCCC_WHY_NOT_VECTORIZE=1 "$ccc" -O2 "${KERNEL_ISA[@]}" -S "$src" -o "$tmp/tr-why.s" 2>"$tmp/tr-why.err"
"$ccc" -O2 "${KERNEL_ISA[@]}" -S "$src" -o "$tmp/tr-quiet.s" 2>"$tmp/tr-quiet.err"

n_hdr=$(grep -cE "$hdr_re" "$tmp/tr-dbg.err" || true)
n_ref=$(grep -cE "$ref_re" "$tmp/tr-dbg.err" || true)
n_nonblank=$(grep -cvE '^[[:space:]]*$' "$tmp/tr-dbg.err" || true)
check_gt0 "trace: header lines under LCCC_DEBUG_VECTORIZE + kernel ISA flags" "${n_hdr:-0}"
check_eq  "trace: one refusal per header line" "$n_hdr" "$n_ref"
check_eq  "trace: the gate prints nothing besides the paired lines" \
          "$n_nonblank" "$((n_hdr + n_ref))"

# Pairing and ORDER: every header must be immediately followed by the refusal
# for the same function, with no stray line in between or after.
pair_err=$(awk '
    /^\[VEC\] Function: / {
        name = $3; sub(/,$/, "", name); pending = name; next
    }
    /: not vectorized: x86 SIMD disabled/ {
        name = $3; sub(/:$/, "", name)
        if (pending == "")      printf "refusal without a header line: %s\n", name
        else if (pending != name) printf "header/refusal name mismatch: %s != %s\n", pending, name
        pending = ""; next
    }
    { printf "stray trace line: %s\n", $0 }
    END { if (pending != "") printf "header without a refusal: %s\n", pending }
' "$tmp/tr-dbg.err")
if [[ -n $pair_err ]]; then
    echo "FAIL: ISA-gate trace pairing/order" >&2
    printf '%s\n' "$pair_err" >&2
    fail=1
fi

# The loop count must be the analysed one, not a placeholder: if the hoist ever
# skipped find_natural_loops under trace, every count would collapse to 0.
max_loops=$(grep -oE 'loops: [0-9]+' "$tmp/tr-dbg.err" | awk '{print $2}' | sort -n | tail -1)
check_gt0 "trace: the natural-loop analysis still runs under trace" "${max_loops:-0}"

check_eq "trace: LCCC_WHY_NOT_VECTORIZE alone prints no header lines" \
         "$(grep -cE "$hdr_re" "$tmp/tr-why.err" || true)" 0
check_eq "trace: both variables agree on the refusal count" \
         "$(grep -cE "$ref_re" "$tmp/tr-why.err" || true)" "$n_ref"
check_eq "trace: no variable, no output" \
         "$(wc -c < "$tmp/tr-quiet.err" | tr -d '[:space:]')" 0

if [[ $fail -ne 0 ]]; then exit 1; fi
echo "vectorizer/FMA x86 ISA emission gates: PASS"
