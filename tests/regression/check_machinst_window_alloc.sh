#!/usr/bin/env bash
# Structural check: the spill-pressure kernel must (a) actually flow through
# the MachInst window pipeline at all ([MI-STREAM] for the function — the
# pre-2026-09-06 loop gate excluded loop-heavy functions entirely), and
# (b) never replay a buffered window through the text path ([MI-FALLBACK]).
#
# Mutation-verified 2026-09-06: the pre-allocator binary fails (a) at the
# old gate default (the function is excluded), and fails (b) under
# CCC_MI_FN_FORCE; reverting the window allocator reintroduces MI-FALLBACK
# windows under force.
set -u
CCC=${CCC:-./target/fastbuild/lccc}
SRC=$(dirname "$0")/machinst_window_alloc_spill_dest.c

OUT=$(CCC_MI_DEBUG=1 CCC_MI_STREAM=1 "$CCC" -O2 -S "$SRC" -o /tmp/mwa_check.$$ 2>&1)
rm -f /tmp/mwa_check.$$
if ! grep -q "MI-STREAM fn=window_alloc_pressure" <<< "$OUT"; then
    echo "FAIL: spill-pressure kernel did not flow through MachInst (loop gate regression?)"
    exit 1
fi
if grep -q "MI-FALLBACK" <<< "$OUT"; then
    echo "FAIL: window allocator replayed a window (MI-FALLBACK present)"
    exit 1
fi
exit 0
