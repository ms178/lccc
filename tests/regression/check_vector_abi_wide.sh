#!/usr/bin/env bash
# AUD-VABI-2: 32-byte vectors across a compiler boundary -- measured, latched.
#
# The sibling gate (check_vector_abi_boundary.sh) pins the 16-byte case, which
# AUD-VABI-1 fixed: a `<4 x i32>` argument or return is ONE %xmm and lccc now
# speaks that.  The 32-byte case is a different rule with the same shape of bug:
# the psABI classifies a vector as SSE + one SSEUP per further eightbyte, so
# WITH AVX ENABLED a 32-byte vector is ONE %ymm register.  lccc's struct rule
# has no SSEUP notion and stops at 16 bytes, so a 32-byte vector falls back to
# the memory convention -- which is what a gcc caller does NOT do the moment
# `-mavx2` is on.  Measured at the commit that added this gate:
#
#   with -mavx2     gcc/gcc        -> `add: 42 42 42 42 42 42 42 42`
#                   lccc/lccc      -> `add: 42 42 42 42 42 42 42 42` (consistent)
#                   lccc + gcc     -> `add: 0 0 0 0 0 0 0 0`   SILENT GARBAGE
#                   gcc + lccc     -> SIGSEGV
#   without -mavx2  all four legs  -> `add: 42 ...`            (memory convention)
#
# The default code-generation baseline of this project is x86-64-v3, i.e. AVX2
# on, so this is not a corner: it is the default configuration, and the failure
# mode on the caller side is a silently wrong answer.
#
# The fix is a %ymm carrier (the 16-byte fix's shape again: one register, a
# wider one), which touches return/argument classification, the prologue,
# location allocation and the call sequence -- a project, not a patch, so this
# gate LATCHES the debt instead of pretending it is fixed:
#
#   * the gcc/gcc reference and lccc/lccc are asserted to be CORRECT -- those
#     are the parts that work and must keep working;
#   * the two cross legs are required to show exactly the documented verdicts
#     (`wrong-values` and `signal`).  A leg that changes -- fixed, or broken in
#     a new way -- FAILS the gate and demands the budget below be updated in
#     the commit that changed it.  A debt that cannot be re-measured is how a
#     worse bug hides behind a known one.
#   * the no-AVX2 configuration must stay green on all four legs, which also
#     proves the comparison in this gate is live (a gate whose diffs never fire
#     passes vacuously).
set -euo pipefail

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
ccc=${CCC:-$repo/target/fastbuild/lccc}
gcc_bin=${GCC:-gcc}
src=$repo/tests/regression/vector_abi_boundary/wide.c
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

[ -x "$ccc" ] || { echo "SKIP: $ccc not built" >&2; exit 0; }
command -v "$gcc_bin" >/dev/null 2>&1 || {
    echo "SKIP: oracle unavailable ($gcc_bin not found)" >&2; exit 0; }

fail() { echo "FAIL: $*" >&2; exit 1; }

# The measured verdict of the two cross-compiler legs in the AVX-enabled
# configuration.  Update ONLY together with the fix (then both are `agree`).
DEBT_LCCC_CALLER=garbage
DEBT_GCC_CALLER=signal

# run_leg <opt> <extra-flags...>: build the four combinations and classify them.
# Prints "<leg-A> <leg-B>" where each is `agree`, `garbage`, or `signal`.
classify() {
    local bin=$1 ref=$2
    local out rc
    set +e
    out=$("$bin" 2>&1)
    rc=$?
    set -e
    if [ "$rc" -ge 128 ]; then
        echo signal
    elif [ "$out" = "$(cat "$ref")" ]; then
        echo agree
    else
        echo garbage
    fi
}

one_config() {
    local tag=$1 opt=$2
    shift 2
    local flags=("$opt" "$@")

    # -Wno-psabi: the no-AVX2 configuration IS the memory convention, and gcc's
    # "AVX vector argument without AVX enabled changes the ABI" note is exactly
    # the property under test there -- the warning is expected, not a finding.
    "$gcc_bin" -Wno-psabi "${flags[@]}" -DCALLEE_ONLY -c "$src" -o "$tmp/callee_gcc_$tag.o"
    "$ccc" "${flags[@]}" -DCALLEE_ONLY -c "$src" -o "$tmp/callee_lccc_$tag.o"
    "$gcc_bin" -Wno-psabi "${flags[@]}" -c "$src" -o "$tmp/caller_gcc_$tag.o"
    "$ccc" "${flags[@]}" -c "$src" -o "$tmp/caller_lccc_$tag.o"

    "$gcc_bin" "${flags[@]}" "$tmp/caller_gcc_$tag.o" "$tmp/callee_gcc_$tag.o" -o "$tmp/ref_$tag"
    "$tmp/ref_$tag" >"$tmp/ref_$tag.out"
    # The oracle's own output must be the arithmetic answer: if gcc/gcc were
    # itself broken, every comparison below would be against garbage.
    grep -q '^add: 42 42 42 42 42 42 42 42$' "$tmp/ref_$tag.out" ||
        { cat "$tmp/ref_$tag.out" >&2;
          fail "$tag: the gcc/gcc reference did not produce the expected answer"; }

    # lccc/lccc: self-consistency. This works today and must not regress.
    "$gcc_bin" "${flags[@]}" "$tmp/caller_lccc_$tag.o" "$tmp/callee_lccc_$tag.o" -o "$tmp/cc_$tag" ||
        fail "$tag: linking the two lccc objects failed"
    "$tmp/cc_$tag" >"$tmp/cc_$tag.out"
    diff -u "$tmp/ref_$tag.out" "$tmp/cc_$tag.out" >&2 ||
        fail "$tag: lccc/lccc disagrees with gcc/gcc (this part works and must keep working)"

    "$gcc_bin" "${flags[@]}" "$tmp/caller_lccc_$tag.o" "$tmp/callee_gcc_$tag.o" -o "$tmp/legA_$tag" ||
        fail "$tag: linking the lccc caller against the gcc callee failed"
    "$gcc_bin" "${flags[@]}" "$tmp/caller_gcc_$tag.o" "$tmp/callee_lccc_$tag.o" -o "$tmp/legB_$tag" ||
        fail "$tag: linking the gcc caller against the lccc callee failed"
    echo "$(classify "$tmp/legA_$tag" "$tmp/ref_$tag.out") $(classify "$tmp/legB_$tag" "$tmp/ref_$tag.out")"
}

for opt in -O0 -O2; do
    read -r a b <<<"$(one_config "avx${opt#-}" "$opt" -mavx2)"

    # The AVX-enabled configuration: the documented debt, exactly.
    [ "$a" = "$DEBT_LCCC_CALLER" ] ||
        fail "$opt -mavx2: lccc caller + gcc callee is '$a', the latched debt is '$DEBT_LCCC_CALLER'.
  If it is now 'agree' the AUD-VABI-2 fix landed: set DEBT_LCCC_CALLER=agree and DEBT_GCC_CALLER=agree here.
  If it changed to something else, the failure mode moved and the reproducer has to be re-measured."
    [ "$b" = "$DEBT_GCC_CALLER" ] ||
        fail "$opt -mavx2: gcc caller + lccc callee is '$b', the latched debt is '$DEBT_GCC_CALLER'.
  If it is now 'agree' the AUD-VABI-2 fix landed: set DEBT_LCCC_CALLER=agree and DEBT_GCC_CALLER=agree here.
  If it changed to something else, the failure mode moved and the reproducer has to be re-measured."
    echo "ok: $opt -mavx2: latched AUD-VABI-2 debt reproduced (lccc caller: $a, gcc caller: $b)"

    # The memory-convention configuration: all four legs must agree. This is
    # also the gate's own liveness proof -- the same diffs that are silent above
    # (because both compilers agree) do fire when the convention matches.
    read -r a b <<<"$(one_config "mem${opt#-}" "$opt")"
    [ "$a" = agree ] ||
        fail "$opt (no -mavx2): lccc caller + gcc callee is '$a', expected 'agree' -- the 32-byte memory convention must keep interoperating"
    [ "$b" = agree ] ||
        fail "$opt (no -mavx2): gcc caller + lccc callee is '$b', expected 'agree' -- the 32-byte memory convention must keep interoperating"
    echo "ok: $opt (no -mavx2): all four combinations agree with gcc/gcc"
done

echo "PASS: AUD-VABI-2 debt latched (32-byte vectors: the AVX %ymm convention is"
echo "      missing, measured on both call directions; the no-AVX2 convention agrees)"
