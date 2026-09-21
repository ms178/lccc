#!/usr/bin/env bash
# Nested-diamond if-conversion + sub-word SELECT demotion gate (session S61).
#
# Pins three fixes that together unlock the whole conditional-clamp family:
#
#   (A) `arm_load_speculation_ok` collects its coverage evidence from the
#       diamond pred's UNIQUE-PREDEDESSOR chain (the blocks that dominate
#       it), not just the pred block — a nested conditional's inner head
#       holds nothing but the compare, so the covering access lives
#       further up.
#
#   (B) `detect_diamond` walks each arm as a single-entry single-exit
#       straight-line REGION, so once the inner level has converted, the
#       outer level's two-block false arm (`head -> inner_merge`) is still
#       recognised and the nested Select forms.
#
#   (C) the block-level SLP vectorizer DEMOTES a promoted sub-word SELECT
#       tree to lane width, recursively: a two-sided clamp promotes the
#       whole tree to `int`, so the outer arm is itself a promoted SELECT.
#       `build_demoted_select_arm` demotes that arm in turn, `Copy` is
#       transparent to `build_pack`, and `Copy` joins the retire fixpoint's
#       feeder kinds so the copy the front end emits for a twice-read C
#       temporary does not strand the load lane's legality.
#
# Contracts checked here:
#
#   1. Runtime clean at -O1/-O2/-O3, and the negative controls SURVIVE:
#      they pass NULL pointers down the path that must not dereference
#      them, so an over-eager speculation gate is a segfault, not a wrong
#      number.
#   2. Tri-config differential: if-conversion on / `CCC_DISABLE_PASSES=
#      if_convert` (the same compiler's branchy reference) / gcc — every
#      check is a value check, so all three must agree line for line.
#   3. Codegen contracts on the runtime battery:
#      a. the clamp family is BRANCH-FREE in its own body (no conditional
#         jump inside the function — the diamonds became selects);
#      b. the straight-line lane clamps actually VECTORIZE (xmm/ymm present);
#      c. the loop clamp vectorizes (the 41x runtime win must not silently
#         regress to the scalar cmov loop);
#      d. the uncovered-load negative controls still BRANCH (they must not
#         have been converted at all);
#      e. the side-effect arms still BRANCH and still write their sink.
#      f. the half-covered store set still branches.
#
# Every contract names its functions explicitly, and a missing symbol is a
# hard failure: an empty body would otherwise read as "zero branches" and
# silently satisfy a must-branch contract.
set -euo pipefail
cd "$(dirname "$0")/../.."
ccc=${LCCC_BIN:-${CCC:-target/fastbuild/lccc}}
bat=tests/regression/bb_slp_nested_ifconv.c
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT

march=""
if "$ccc" -march=x86-64-v3 -E -x c /dev/null -o /dev/null 2>/dev/null; then
    march="-march=x86-64-v3"
fi

want="bb_slp_nested_ifconv: all pass (0 fails)"

# ── 1. runtime, if-conversion on, at three optimisation levels ─────────
for opt in -O1 -O2 -O3; do
    "$ccc" $opt $march "$bat" -o "$td/rt_on"
    out_on=$("$td/rt_on")
    if [ "$out_on" != "$want" ]; then
        echo "FAIL: nested_ifconv runtime ($opt, if-conversion on): $out_on"
        exit 1
    fi
done

# ── 2. tri-config differential ─────────────────────────────────────────
CCC_DISABLE_PASSES=if_convert "$ccc" -O2 $march "$bat" -o "$td/rt_off"
out_off=$("$td/rt_off")
if [ "$out_off" != "$want" ]; then
    echo "FAIL: nested_ifconv runtime (if-conversion disabled): $out_off"
    exit 1
fi
if command -v gcc >/dev/null 2>&1; then
    gcc -O2 $march "$bat" -o "$td/rt_gcc" 2>/dev/null
    out_gcc=$("$td/rt_gcc")
    if [ "$out_gcc" != "$want" ]; then
        echo "FAIL: nested_ifconv lccc vs gcc differential"
        echo "  lccc: $out_on"
        echo "  gcc : $out_gcc"
        exit 1
    fi
fi

# ── 3./4. codegen contracts ────────────────────────────────────────────
"$ccc" -O2 $march -S "$bat" -o "$td/bat.s"

asm=""
body() { # body <function> — the function's own instruction stream
    awk -v fn="$1:" '$0 ~ "^"fn {inb=1} inb {print} inb && /^\.size/ {exit}' "$asm"
}
require_fn() { # require_fn <function> — the symbol must exist in $asm
    if ! grep -qE "^$1:" "$asm"; then
        echo "FAIL: function '$1' is absent from $(basename "$asm")"
        echo "      (a missing symbol would silently read as zero branches"
        echo "       and satisfy a must-branch contract)"
        exit 1
    fi
}
count_branches() { grep -cE '^[[:space:]]*j[a-z]+[[:space:]]' || true; }
count_vec()      { grep -cE '%xmm|%ymm' || true; }

must_be_branchless_and_vector() {
    require_fn "$1"
    local b nb nv
    b=$(body "$1")
    nb=$(printf '%s\n' "$b" | count_branches)
    nv=$(printf '%s\n' "$b" | count_vec)
    if [ "$nb" -ne 0 ]; then
        echo "FAIL: $1 still has $nb conditional branches (diamonds not converted)"
        exit 1
    fi
    if [ "$nv" -eq 0 ]; then
        echo "FAIL: $1 did not vectorize (no xmm/ymm in its body)"
        exit 1
    fi
}
must_branch() {
    require_fn "$1"
    if [ "$(body "$1" | count_branches)" -eq 0 ]; then
        echo "FAIL: $2"
        exit 1
    fi
}

# (a)+(b) the runtime battery's straight-line lane clamps.
asm="$td/bat.s"
for fn in clamp_u8x16 clamp_i32x4 clamp3_i32x4 clamp_dyn_i32x4 \
          clamp_u16x8 clamp_i16x8; do
    must_be_branchless_and_vector "$fn"
done

# The TEMPORARY spelling of the same family: the inner compare stays
# promoted and the twice-read operand is a `Copy` of the load, so this is a
# distinct path through `build_demoted_select_arm` and `build_pack`.
for fn in clamp_lo_u8x16 clamp_tmp_u8x16 clamp_tmp_i8x16 clamp_tmp_u16x8 \
          clamp_tmp_i16x8 clamp_tmp_i32x4; do
    must_be_branchless_and_vector "$fn"
done

# (c) the loop clamp must vectorize — this is the 41x runtime win.
require_fn clamp_loop_u8
if [ "$(body clamp_loop_u8 | count_vec)" -eq 0 ]; then
    echo "FAIL: clamp_loop_u8 did not vectorize (loop clamp regressed to scalar)"
    exit 1
fi

# (d) the uncovered-load negative controls must NOT have been converted:
# their arm load is not provably dereferenceable on the other path, so the
# diamonds must survive as branches.
must_branch half_guard \
    "half_guard was if-converted despite an uncovered arm load
      (speculating it dereferences NULL — soundness regression)"
must_branch nested_uncovered \
    "nested_uncovered was if-converted despite an uncovered arm load
      (speculating it dereferences NULL — soundness regression)"

# (e) the side-effect arms must stay branchy (their arms write a volatile).
must_branch side_effect_arms \
    "side_effect_arms was if-converted despite volatile side effects"
must_branch volatile_arm \
    "volatile_arm was if-converted despite volatile side effects"
must_branch volatile_store_arm \
    "volatile_store_arm was if-converted despite volatile side effects"

# (d2) the half-covered STORE set must not be converted either.
must_branch half_store \
    "half_store was if-converted despite a half-covered store set"

echo "PASS: bb_slp_nested_ifconv (runtime + tri-config differential + codegen contracts)"
