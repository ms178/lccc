#!/usr/bin/env bash
# GNU vector-extension (`vector_size`) whole-vector lowering: runtime
# differential against GCC plus per-function code-quality ceilings.
#
# Part 1 -- runtime.  `vector_ext/matrix.c` (every lane type x {16, 32}
# bytes x {add sub mul div and or xor shl shr neg not == <}, loops,
# splats, lane access, ternaries, assignments, nine-argument functions,
# struct/union wrappers, sign-of-zero negation, call-through-GCC) is
# compiled by lccc and by GCC at every ISA/opt level below; both objects
# are symbol-prefixed and linked into one `driver.c` (GCC-built) that feeds
# identical pseudo-random inputs to each pair and compares the results
# bit for bit (284 checks).  The cross-compiler link also proves the
# psABI vector carriers (%xmm / %ymm / memory, the ninth argument, the
# ymm0 return, the sret struct) agree with GCC in both directions
# (`viacall` calls a GCC-built callee from lccc code).
#
# Regressions pinned here (all observed, all fixed in the same change):
#   * `return -a` on a float-lane vector negated the ALLOCA POINTER
#     (`lower_unary_op` Neg arm) -- a crash on every float/double negate;
#   * `r = c ? a : b` over 32-byte vectors at -mno-avx selected the wrong
#     object for every `c`: the over-aligned alloca address staging
#     (`addq $31; andq $-32`) clobbered the flags between the test and the
#     cmov (`emit_select_impl`);
#   * a 32-byte vector argument below the AVX ABI level (MEMORY class) lost
#     its parameter slot while the prologue still captured through it
#     (stack layout vs prologue predicate drift) -- an internal error on
#     every such function at -O2;
#   * the home-to-%ymm0 move of a register-valued vector return was deleted
#     by the FP liveness oracle (`LCCC_RET_XMM` did not name bank 0).
#
# Part 2 -- code quality at -O2 -mavx2 (the host's level), lccc only, so
# the ceilings do not drift with the GCC version: instruction counts for
# the shapes the whole-vector lowering owns.  Before this lowering `add8`
# was 37 instructions (GCC: 2); the ceilings below are the measured counts
# at the time of writing, each AT GCC PARITY unless noted.
set -euo pipefail
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
ccc=${CCC:-$root/target/fastbuild/lccc}
gcc_bin=${GCC:-gcc}
[ -x "$ccc" ] || { echo "FAIL: compiler not built: $ccc" >&2; exit 1; }
command -v "$gcc_bin" >/dev/null || { echo "FAIL: no GCC ($gcc_bin)" >&2; exit 1; }
command -v objcopy >/dev/null || { echo "FAIL: objcopy not found" >&2; exit 1; }
case $(uname -m) in
    x86_64) ;;
    *) echo "SKIP: x86-64 only"; exit 0 ;;
esac
src=$root/tests/regression/vector_ext/matrix.c
drv=$root/tests/regression/vector_ext/driver.c
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

fails=0
fail() {
    echo "FAIL: $*" >&2
    fails=$((fails + 1))
}

# ---------------------------------------------------------------- Part 1
# Execution levels follow the host CPU (the assembly-only Part 2 does not).
levels=("-O0" "-O1" "-O2" "-O2 -mno-avx")
cpu_has() { grep -qw "$1" /proc/cpuinfo 2>/dev/null; }
cpu_has avx && levels+=("-O2 -mavx")
if cpu_has avx2 && cpu_has fma; then
    # `-ffp-contract=on` is deliberately NOT a runtime level: GCC 16 only
    # implements in-expression contraction for scalars (its vector `a*b+c`
    # stays vmulps+vaddps), while lccc — like Clang — contracts vector
    # expressions too, so the single-rounding results differ.  Part 3
    # checks the `on`/`off` policy on the assembly instead.
    levels+=("-O2 -mavx2" "-O3 -mavx2 -mfma" "-Os -mavx2" "-O2 -mavx2 -mno-fma"
             "-O2 -mavx2 -mfma -ffp-contract=off")
fi
cpu_has avx512f && levels+=("-O2 -mavx512f")
# GCC's ISA must match the one lccc ASSUMES, not merely the flags: lccc's
# documented default-flag baseline is x86-64-v3 for CODEGEN
# (`src/backend/x86/isa.rs`), so FMA3 contraction is on unless a level
# denies it (`-mno-fma`, or `-mno-avx` since `fma ⇒ avx`) or pins an
# `-march=`.  The 32-byte vector ABI, by contrast, follows the EXPLICIT
# flags on both compilers (memory without `-mavx*`, %ymm with), so GCC may
# only receive `-mfma` (which implies `-mavx`, i.e. the %ymm ABI) at levels
# that already name an AVX ISA.  At the remaining levels GCC cannot fuse at
# all; there lccc runs with `-ffp-contract=off` so the differential compares
# equal roundings (lccc's fusion at those levels is covered by Part 3).
gcc_isa_flags() {
    case " $1 " in
        *" -mno-fma "* | *" -mno-avx "* | *" -mfma "* | *" -march="*) echo "$1" ;;
        *" -mavx"*) echo "$1 -mfma" ;;
        *) echo "$1" ;;
    esac
}
lccc_isa_flags() {
    case " $1 " in
        *" -mno-fma "* | *" -mno-avx "* | *" -mfma "* | *" -march="* | *" -mavx"*) echo "$1" ;;
        *) echo "$1 -ffp-contract=off" ;;
    esac
}
runs=0
for flags in "${levels[@]}"; do
    gflags=$(gcc_isa_flags "$flags")
    lflags=$(lccc_isa_flags "$flags")
    # The IR verifier runs after every pass (`CCC_VERIFY_IR=1`) and REPORTS
    # rather than aborts, so its output is a hard failure here: a
    # use-before-def that still happens to execute correctly on one input
    # (the SLP shared-load schedule, `ho_*_al2`) must not pass as green.
    # shellcheck disable=SC2086
    if ! CCC_VERIFY_IR=1 "$ccc" $lflags -c "$src" -o "$tmp/m.l.o" 2>"$tmp/l.err"; then
        fail "lccc $flags: compile error: $(head -c 300 "$tmp/l.err")"
        continue
    fi
    if grep -q 'ir-verify' "$tmp/l.err"; then
        fail "lccc $flags: IR verifier report: $(grep -m1 'ir-verify' "$tmp/l.err")"
        continue
    fi
    # shellcheck disable=SC2086
    "$gcc_bin" $gflags -w -Wno-psabi -c "$src" -o "$tmp/m.g.o"
    objcopy --prefix-symbols=L_ "$tmp/m.l.o"
    objcopy --prefix-symbols=G_ "$tmp/m.g.o"
    # shellcheck disable=SC2086
    "$gcc_bin" -O0 $gflags -w -Wno-psabi "$drv" "$tmp/m.l.o" "$tmp/m.g.o" -o "$tmp/rt.bin"
    if ! out=$("$tmp/rt.bin" 2>&1); then
        fail "runtime differential at $flags:"$'\n'"$out"
        continue
    fi
    case $out in
        *" checks passed") runs=$((runs + 1)) ;;
        *) fail "runtime differential at $flags: unexpected output: $out" ;;
    esac
done
[ "$runs" -gt 0 ] || fail "no runtime differential ran"

# ---------------------------------------------------------------- Part 2
"$ccc" -O2 -mavx2 -S "$src" -o "$tmp/m.s"
count() {
    # Instructions in function $1 (lines after its label up to .cfi_endproc,
    # excluding directives, labels, comments and blank lines).
    awk -v fn="$1" '
        $0 == fn ":" { on = 1; next }
        on && /^\s*\.cfi_endproc/ { exit }
        on {
            line = $0; sub(/^[ \t]+/, "", line)
            if (line == "" || line ~ /^[.#]/ || line ~ /:$/) next
            n++
        }
        END { print n + 0 }' "$tmp/m.s"
}
ceiling() {
    local fn=$1 max=$2 n
    n=$(count "$fn")
    if [ "$n" -gt "$max" ]; then
        fail "$fn: $n instructions at -O2 -mavx2 (ceiling $max)"
    fi
}
# Two-operand arithmetic: one packed op + ret (GCC: 2).
for fn in add_v32f sub_v32f mul_v32f div_v32f add_v32d mul_v32d add_v32i sub_v32i mul_v32i \
          and_v32i or_v32i xor_v32i add_v16f mul_v16f add_v16i xor_v16i add_v32s add_v16c \
          add_v32c sub_v32l and_v32l; do
    ceiling "$fn" 2
done
# Negate / not: constant load folded into the xor + ret (GCC: 2).
for fn in neg_v32f neg_v32d neg_v16f negz negz2 not_v32i; do ceiling "$fn" 3; done
# Per-lane variable shifts: one vpsllvd/vpsrlvd/vpsravd/vpsllvq/vpsrlvq
# + ret (GCC: 2); scalar-count forms broadcast the count (GCC: 4, it
# shifts by an xmm count without the broadcast).
for fn in shl_v32i shr_v32i shl_v32l sh_vv_u sh_vv_ul sh_mixsign sh_mixsign2 \
          shl_v16i shr_v16i shl_v16l sh_vv_u4 sh4_vv_shl_l; do ceiling "$fn" 2; done
ceiling sh_chain 3
ceiling sh_then_add 3
ceiling sh4_chain 4
ceiling sh4_vs_sar 6
for fn in sh_vs_shl sh_vs_sar sh_vs_shr sh_vs_shl_l sh_vs_shr_ul; do ceiling "$fn" 6; done
if ! awk '$0 == "shr_v32i:" {on=1; next} on && /\.cfi_endproc/ {exit} on' "$tmp/m.s" |
    grep -q 'vpsravd'; then
    fail "shr_v32i: expected a per-lane vpsravd at -O2 -mavx2"
fi
# Compare masks (GCC: 2).
for fn in eq_v32i lt_v32i eq_v32f lt_v32f lt_v16f; do ceiling "$fn" 2; done
# DEBT (VEX-128-INT-1): the 128-bit integer compare/arith emitters stage
# through legacy destructive SSE forms under AVX (GCC: 2 / 4).
ceiling eq_v16i 4
ceiling mix_sab4 7
# Scalar/vector interleave: broadcast + fma + ret (GCC: 3).
ceiling mix_sab 3
# Parameter passthrough (GCC: 1).
ceiling ident 1
ceiling ident2 1
# The ternary over two register arms / lvalue arms / a mask select
# (GCC: 6 / 6 / 6 / 3; the extra moves are the xmm0/xmm1 scratch
# reservation and the select's flag-preserving arm staging).
ceiling cond_v32f 9
ceiling cond_v16i 9
ceiling tern_lv 8
ceiling tern_assign 10
ceiling sel_mask 7
# Streaming loop `r[i] = a[i]*b[i] + r[i]`: the body is load/mul/add/store
# + control (GCC: 14 / 13 / 14 incl. its prologue).
ceiling loop_v32f 16
ceiling loop_v16f 15
ceiling loop_v32i 16
# Shapes still on the per-lane scalar path, ratcheted at their measured
# counts so the memory-object cost model cannot regress unnoticed
# (GCC: 2 / 11 / 11 / 15; see FOLLOWUP "vector-ext scalar residue").
ceiling sadd 18        # struct-wrapped compound assign: lane stores + memcpy
ceiling s4i_add 26     # struct-wrapped 128-bit add
ceiling tern_se 53     # ternary whose arms are calls (two sret temporaries)
ceiling viacall 30     # by-value call into a GCC-built callee + return
ceiling shr_v32l 23    # signed 64-bit lanes: no vpsravq below AVX-512VL
ceiling shr_v16l 15    # (GCC: 6 — it emulates with vpsrlvq + sign fixup)
ceiling eq_v32l 41     # 64-bit lane compare (no vpcmpeqq/vpcmpgtq lowering yet)
ceiling mul_v32c 136   # byte multiply (no pmullw/pack expansion yet)
# FMA contraction (`vecfma`): `a*b + c` and its sign variants are one
# vfmadd/vfmsub/vfnmadd + ret at 256 and 128 bits (GCC: 2).  The squared
# and self-addend forms coalesce onto the dying multiplicand (GCC: 2; ours
# is 1 — no copy to the return register is needed when the handoff lands
# in %ymm0).  The two-deep chain keeps the first FMA's parameter prestore
# (GCC: 3 — DEBT: copy-in rename when the source register is dead).
for fn in fma_f8 fma_f8r fnma_f8 fms_f8 fma_d4 fnma_d4 fms_d4 \
          fma_f4 fnma_f4 fms_f4 fma_d2 fnma_d2; do
    ceiling "$fn" 2
done
ceiling fma_sq 2
ceiling fma_sq4 4      # DEBT (VEX-128-INT-1 sibling): legacy movdqa staging
ceiling fma_self 2
ceiling fma_chain 4
ceiling fma_two 6      # m = a*b reused by + and -: both contract (GCC: 4)
ceiling fma_nouse 4
# Streaming FMA loop `r[i] = a[i]*b[i] + c[i]` (GCC: 14).
ceiling fma_loop 17
# Loop-carried accumulators: register-resident, zero seed in a register,
# no frame (GCC: 19 each — it peels/aligns the loop).  `acc_v32i` keeps a
# frame (vpxor seed still slot-staged — DEBT) at 24.
for fn in acc_v32f acc_v16f acc_v32d; do ceiling "$fn" 12; done
ceiling acc_v16c 13
ceiling acc_v32i 24
# `s += a[i]*b[i]` reduction: NOT fused (GCC generic tuning and lccc's
# scalar gate agree — the multiply stays off the serial add chain).
ceiling fma_acc 16
if awk '$0 == "fma_acc:" {on=1; next} on && /\.cfi_endproc/ {exit} on' "$tmp/m.s" |
    grep -q 'vfm'; then
    fail "fma_acc: loop-carried accumulator update was fused"
fi
if ! awk '$0 == "fma_f8:" {on=1; next} on && /\.cfi_endproc/ {exit} on' "$tmp/m.s" |
    grep -q 'vfmadd'; then
    fail "fma_f8: expected a fused vfmadd at -O2 -mavx2 (default -ffp-contract=fast)"
fi
# Memory-object shapes must not realign per access.
for fn in add_v32f cond_v32f loop_v32f sel_mask mix_sab; do
    if awk -v fn="$fn" '$0 == fn ":" {on=1; next} on && /\.cfi_endproc/ {exit} on' "$tmp/m.s" |
        grep -q 'andq \$-32'; then
        fail "$fn: per-access over-aligned realignment (andq \$-32) survives"
    fi
done
# The 256-bit return must come back in %ymm0 with no stack round trip.
if ! awk '$0 == "add_v32f:" {on=1; next} on && /\.cfi_endproc/ {exit} on' "$tmp/m.s" |
    grep -q 'vaddps %ymm1, %ymm0, %ymm0'; then
    fail "add_v32f: expected 'vaddps %ymm1, %ymm0, %ymm0'"
fi

# ---------------------------------------------------------------- Part 3
# `-ffp-contract` policy on the assembly.  `off`: no fused op anywhere in
# the matrix.  `on`: contraction stays inside one expression — `fma_f8`
# (`a*b + c`) fuses, `fma_two` (`m = a*b;` reused by two later statements)
# must not.
"$ccc" -O2 -mavx2 -mfma -ffp-contract=off -S "$src" -o "$tmp/off.s"
if grep -q '^\s*vfn\?m\(add\|sub\)' "$tmp/off.s"; then
    fail "-ffp-contract=off: fused multiply-add survives"
fi
"$ccc" -O2 -mavx2 -mfma -ffp-contract=on -S "$src" -o "$tmp/on.s"
if ! awk '$0 == "fma_f8:" {on=1; next} on && /\.cfi_endproc/ {exit} on' "$tmp/on.s" |
    grep -q 'vfmadd'; then
    fail "-ffp-contract=on: fma_f8 (single expression) not contracted"
fi
if awk '$0 == "fma_two:" {on=1; next} on && /\.cfi_endproc/ {exit} on' "$tmp/on.s" |
    grep -q 'vfm'; then
    fail "-ffp-contract=on: fma_two contracted across statements"
fi

if [ "$fails" -ne 0 ]; then
    echo "FAIL: $fails failure(s)" >&2
    exit 1
fi
echo "PASS: vector-extension codegen ($runs runtime levels, ceilings held, contract policy)"
