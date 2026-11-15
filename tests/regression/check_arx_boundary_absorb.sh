#!/usr/bin/env bash
# vec_arx boundary absorption (entry copy + feed-forward): the fix for
# the 2.26x chacha20_block regression vs GCC on EPYC 9V45 (Zen 5).
#
# Mechanism gates (check_arx_boundary_absorb.py): element-wise entry
# pack, the one-sided runtime overlap guard, and the branch-free vector
# feed-forward with the state spill demoted to the cold path.  A
# control build with CCC_DISABLE_PASSES=vec_arx must FAIL the gates, so
# the predicates can never rot into vacuity.
#
# Semantics gates: every overlap region the runtime guard routes
# (disjoint / exact / above / below / far) is executed 1024 times with
# clobber feedback and compared bit-for-bit against a GCC build AND
# against the pass-disabled lccc build, at every optimization level the
# pass changes behaviour at (-O2, -O3) plus the levels it must leave
# alone (-O0, -Os).
set -euo pipefail
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
CCC=${CCC:-$here/../../target/fastbuild/lccc}
GCC=${GCC_BIN:-gcc}
work=$(mktemp -d "${TMPDIR:-/tmp}/arx-absorb.XXXXXX")
trap 'rm -rf "$work"' EXIT HUP INT TERM

src=$here/arx_boundary_absorb.c
bench=$here/../benchmark/programs/chacha20_block.c

[ -x "$CCC" ] || { echo "FAIL: lccc binary missing: $CCC"; exit 1; }

# ── Mechanism: assembly witnesses on the benchmark TU (-O2 baseline). ──
"$CCC" -O2 -S "$bench" -o "$work/absorbed.s"
CCC_DISABLE_PASSES=vec_arx "$CCC" -O2 -S "$bench" -o "$work/control.s"
python3 "$here/check_arx_boundary_absorb.py" "$work/absorbed.s" absorbed chacha20_core
python3 "$here/check_arx_boundary_absorb.py" "$work/control.s" control chacha20_core

# The differential fixture must hit the same absorbed shape.
"$CCC" -O2 -S "$src" -o "$work/fixture.s"
python3 "$here/check_arx_boundary_absorb.py" "$work/fixture.s" absorbed chacha20_core

# ── Semantics: overlap-region differential vs GCC and vs the control. ─
"$GCC" -O2 "$src" -o "$work/ref"
"$work/ref" > "$work/ref.out"
if ! grep "^disjoint " "$work/ref.out" > /dev/null; then
    echo "FAIL: reference run produced no scenario lines"
    exit 1
fi
for flags in '-O0' '-O2' '-O3' '-Os' '-O2 -march=x86-64-v3'; do
    # shellcheck disable=SC2086
    CCC_VERIFY_IR=abort "$CCC" $flags "$src" -o "$work/run"
    "$work/run" > "$work/run.out"
    if ! cmp -s "$work/run.out" "$work/ref.out"; then
        echo "FAIL [$flags]: lccc diverges from gcc on guard-region semantics"
        diff "$work/ref.out" "$work/run.out" || :
        exit 1
    fi
    # shellcheck disable=SC2086
    CCC_DISABLE_PASSES=vec_arx "$CCC" $flags "$src" -o "$work/ctl"
    "$work/ctl" > "$work/ctl.out"
    if ! cmp -s "$work/ctl.out" "$work/ref.out"; then
        echo "FAIL [$flags]: pass-disabled control diverges from gcc"
        diff "$work/ref.out" "$work/ctl.out" || :
        exit 1
    fi
    echo "ok   [$flags]: all five overlap regions match gcc (absorbed and control)"
done

echo "ARX boundary absorption: PASS"
