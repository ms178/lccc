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
# Part 2 -- code quality at -O2 -march=x86-64-v3 (opt-in AVX2 level), lccc only, so
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

# i686 (-m32) runtime differential: the whole-vector lowering publishes the
# 16-byte SSE2 ceiling on i686 (pipeline.rs, `resolved_i686_sse2`), so every
# 16-byte matrix shape must stay bit-identical to GCC in 32-bit code.  The
# 32-byte shapes keep their per-lane scalar fallback at ceiling 16 (both
# compilers agree: GCC's default -m32 baseline has no AVX).  Skipped loudly
# when the environment has no 32-bit libc headers (host multilib or the
# isolated sysroot via LCCC_SYSROOT) — CI and the sysroot-provisioned
# sandbox both take the measuring path.
if printf '#include <stdio.h>\n' | "$ccc" -m32 -fsyntax-only -x c - >/dev/null 2>&1; then
    levels+=("-m32 -O0" "-m32 -O1" "-m32 -O2" "-m32 -O2 -mno-avx" "-m32 -Os")
else
    echo "SKIP: i686 runtime differential - no 32-bit libc headers in this environment"
fi
# Optional, explicit supplemental oracle configurations. Defaults retain the
# complete host-supported matrix; code-quality/contraction checks always run.
# Example: GCC=clang-23 "$0" --runtime-level "-O2 -mavx2 -ffp-contract=off"
if (( $# )); then
    levels=()
    while (( $# )); do
        if [[ $1 != --runtime-level || $# -lt 2 || -z $2 ]]; then
            echo "usage: $0 [--runtime-level 'compiler flags']..." >&2
            exit 2
        fi
        levels+=("$2")
        shift 2
    done
fi
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
    # Pin reference ISA/contraction defaults. Clang implies FMA from
    # -mavx512f and defaults to in-expression contraction, unlike GCC.
    # This does not make incompatible vector ABIs or permissible FP
    # contraction decisions interchangeable across compiler families.
    local flags=$1
    case " $flags " in
        *" -mfma "*) ;;
        *) flags+=" -mno-fma" ;;
    esac
    case " $flags " in
        *" -ffp-contract="*) ;;
        *) flags+=" -ffp-contract=fast" ;;
    esac
    echo "$flags"
}
lccc_isa_flags() {
    case " $1 " in
        *" -mno-fma "* | *" -mno-avx "* | *" -mfma "* | *" -march="* | *" -mavx"*) echo "$1" ;;
        *) echo "$1 -ffp-contract=off" ;;
    esac
}
# Prefix the matrix's OWN symbols (its definitions, and the two helpers
# the driver defines per side: `sink`, `call8`) so the lccc and GCC twins
# coexist in one executable.  Not a blanket `--prefix-symbols`: a GCC
# configured with `--enable-default-ssp` (Debian's packages, hence the
# hosted runner) emits `__stack_chk_fail` for the vector locals, and
# `memcpy`/`memset` may appear at -O0 — a prefixed runtime symbol is an
# undefined reference and the link fails before a single check runs.
prefix_symbols() {
    local pfx=$1 obj=$2
    nm --defined-only -g "$obj" | awk -v p="$pfx" 'NF == 3 { print $3, p $3 }' >"$tmp/syms"
    printf 'sink %ssink\ncall8 %scall8\n' "$pfx" "$pfx" >>"$tmp/syms"
    objcopy --redefine-syms="$tmp/syms" "$obj"
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
    prefix_symbols L_ "$tmp/m.l.o"
    prefix_symbols G_ "$tmp/m.g.o"
    # shellcheck disable=SC2086
    "$gcc_bin" -O0 $gflags -w -Wno-psabi "$drv" "$tmp/m.l.o" "$tmp/m.g.o" -o "$tmp/rt.bin"
    if ! out=$("$tmp/rt.bin" 2>&1); then
        fail "runtime differential at $flags:"$'\n'"$out"
        continue
    fi
    case $out in
        *" checks passed"*) runs=$((runs + 1)) ;;
        *) fail "runtime differential at $flags: unexpected output: $out" ;;
    esac
done
[ "$runs" -gt 0 ] || fail "no runtime differential ran"

# ---------------------------------------------------------------- Part 2
"$ccc" -O2 -march=x86-64-v3 -S "$src" -o "$tmp/m.s"
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
        fail "$fn: $n instructions at -O2 -march=x86-64-v3 (ceiling $max)"
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
    fail "shr_v32i: expected a per-lane vpsravd at -O2 -march=x86-64-v3"
fi
# Compare masks (GCC: 2): every lane width at both register widths, the
# 128-bit forms through the VEX.128 three-operand path.
for fn in eq_v32i lt_v32i eq_v32f lt_v32f lt_v16f eq_v16i lt_v16i eq_v16l lt_v16l \
          eq_v32l lt_v32l eq_v16s lt_v16s eq_v16c lt_v16c \
          cq_lt_s cq_gt_s cq4_lt_s cq_eq_u; do ceiling "$fn" 2; done
# le / ne / unsigned orderings (counts include `ret`): min+eq (3),
# eq/gt+invert (4), bias+gt (4), qword le.u = bias+gt+invert (6); the
# swapped `>=` form pays one RA handoff copy (7).  GCC: 3-6.
for fn in cd4_le_s cd4_le_u cb4_le_s; do ceiling "$fn" 3; done
for fn in cq_le_s cq_ne_s cq4_ne_s cd4_ne_s cw4_ne_s cq_lt_u cq4_lt_u cd4_lt_u; do ceiling "$fn" 4; done
for fn in cq4_le_u cq_le_u; do ceiling "$fn" 6; done
ceiling cq_ge_u 7
ceiling cq_sel 3       # (m & x) | (~m & y) -> vpcmpgtq + vpblendvb + ret (GCC: 3)
# Scalar splats (`(T){} + x`): the integer zero identity folds, the
# broadcast is homed and returned directly (GCC: 3).  FP keeps the
# `{} + x` add (not an identity for -0.0): 4.
for fn in splat_v32i splat_v32c splat_v32s splat_v32l splat_v16i splat_v16c splat_v16s splat_v16l; do ceiling "$fn" 3; done
for fn in splat_v32f splat_v32d splat_v16f splat_v16d; do ceiling "$fn" 4; done
# A scalar integer constant operand is a `.rodata` lane constant (GCC: 4).
ceiling splat_r 2
# Whole-register `__builtin_memcpy` load/store idioms (GCC: 2).  The byte/
# word 128-bit shapes are register-homed like the dword ones.
for fn in ld_v32f ld_v32i ld_v32c ld_v16f ld_v16i ld_v16c ld_v16s \
          st_v32f st_v32i st_v32c st_v32s st_v16f st_v16i st_v16c st_v16s; do ceiling "$fn" 2; done
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
ceiling sel_mask 3     # vcmpps + vpblendvb + ret (GCC: 3)
# Streaming loop `r[i] = a[i]*b[i] + r[i]`: the body is load/mul/add/store
# + control (GCC: 14 / 13 / 14 incl. its prologue).
ceiling loop_v32f 16
ceiling loop_v16f 15
ceiling loop_v32i 16
# Shapes still on the per-lane scalar path, ratcheted at their measured
# counts so the memory-object cost model cannot regress unnoticed
# (GCC: 2 / 11 / 11 / 15; see FOLLOWUP "vector-ext scalar residue").
# MEMORY-class incoming homes avoid capture copies and per-use alignment.
ceiling cf_wrap_phi 10  # was 32; GCC: 6
ceiling nine 12         # ninth vector is stack-passed; was 20; GCC: 9
ceiling nine4 11        # was 17; GCC: 9
ceiling sadd 2         # single-member vector record: returned like the bare vector (GCC: 2)
ceiling s4i_add 16     # struct-wrapped 128-bit add
ceiling tern_se 53     # ternary whose arms are calls (two sret temporaries)
ceiling viacall 30     # by-value call into a GCC-built callee + return
ceiling shr_v32l 6     # signed 64-bit lanes: vpsrlvq + sign fix-up (xor/sub), GCC: 6
ceiling shr_v16l 6

# ---------------------------------------------- Part 2b: -m32 SSE2 ceilings
# The i686 packed lowering (SSE2 ceiling, stack-slot homing) counts its own
# measured shapes; GCC's default -m32 baseline (`-march=x86-64` minus the
# 64-bit bit) reaches SSE2 too, so every witness below is a `p*` op that the
# per-lane scalar form used to pay inline (173 adds -> 5 class, measured in
# the backlog table).  Skipped with Part 1 when no 32-bit libc headers exist.
if printf '#include <stdio.h>\n' | "$ccc" -m32 -fsyntax-only -x c - >/dev/null 2>&1; then
    if ! "$ccc" -m32 -O2 -S "$src" -o "$tmp/m32.s" 2>"$tmp/m32.err"; then
        fail "lccc -m32 -O2 -S: $(head -c 300 "$tmp/m32.err")"
    else
        # Positive packed witnesses: the 16-byte integer/FP binops, compares
        # and the SSE2 integer multiply expansion must lower to packed ops.
        for spec in             'add_v16i:paddd' 'sub_v16i:psubd' 'and_v16i:pand' 'or_v16i:por' \
            'xor_v16i:pxor' 'add_v16c:paddb' 'add_v16s:paddw' 'mul_v16i:pmuludq' \
            'add_v16f:addps' 'div_v16f:divps' 'add_v16d:addpd' \
            'eq_v16i:pcmpeqd' 'lt_v16i:pcmpgtd' 'eq_v16f:cmpps' \
            'splat_v16i:pshufd' 'ld_v16i:movups' 'st_v16i:movups'; do
            fn=${spec%%:*}; want=${spec##*:}
            if ! awk -v fn="$fn:" '$0 == fn {on=1; next} on && /^\s*\.cfi_endproc/{exit} on' "$tmp/m32.s" |
                    grep -q "$want"; then
                fail "-m32 $fn: expected $want in the packed lowering (per-lane scalarization regression)"
            fi
        done
        # Instruction ceilings on the same shapes the x86-64 Part 2 pins
        # (measured with this change; GCC's default -m32 baseline: same op
        # counts plus its own prologue differences).
        for fn in add_v16i sub_v16i and_v16i or_v16i xor_v16i add_v16c add_v16s \
                  add_v16f sub_v16f mul_v16f div_v16f add_v16d div_v16d; do
            n=$(awk -v fn="$fn:" '$0 == fn {on=1; next} on && /^\s*\.cfi_endproc/{exit} on {line=$0; sub(/^[ \t]+/,"",line); if (line=="" || line ~ /^[.#]/ || line ~ /:$/) next; n++} END {print n+0}' "$tmp/m32.s")
            # The slot-homed staging costs a bounded number of movups over
            # the bare two-op form; pin the measured envelope (never the
            # 173-instruction scalar era).
            if [ "$n" -gt 44 ]; then
                fail "-m32 $fn: $n instructions at -O2 (packed ceiling 44)"
            fi
        done
        n=$(awk -v fn="mul_v16i:" '$0 == fn {on=1; next} on && /^\s*\.cfi_endproc/{exit} on {line=$0; sub(/^[ \t]+/,"",line); if (line=="" || line ~ /^[.#]/ || line ~ /:$/) next; n++} END {print n+0}' "$tmp/m32.s")
        if [ "$n" -gt 50 ]; then
            fail "-m32 mul_v16i: $n instructions at -O2 (SSE2 pmuludq expansion ceiling 50)"
        fi
        # The compare matrix and shifts must not scalarize either.
        for fn in eq_v16i lt_v16i eq_v16s lt_v16s shl_v16i shr_v16i; do
            n=$(awk -v fn="$fn:" '$0 == fn {on=1; next} on && /^\s*\.cfi_endproc/{exit} on {line=$0; sub(/^[ \t]+/,"",line); if (line=="" || line ~ /^[.#]/ || line ~ /:$/) next; n++} END {print n+0}' "$tmp/m32.s")
            if [ "$n" -gt 44 ]; then
                fail "-m32 $fn: $n instructions at -O2 (compare/shift ceiling 44)"
            fi
        done
    fi
fi
# Byte multiply through the word family: pmullw + mask + two shifts +
# pmullw + shift + or (GCC: 10 / 12).
ceiling mul_v32c 8
ceiling mul_v16c 8
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
ceiling fma_chain 3   # packed FMA prefix coalesces a dying multiplier; GCC: 3
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
    fail "fma_f8: expected a fused vfmadd at -O2 -march=x86-64-v3 (default -ffp-contract=fast)"
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
