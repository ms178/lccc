#!/usr/bin/env bash
# Structural check for the wide-copy-of-small-slot window class.
#
# accumulator_reassociation.c is the corpus trip-wire for the class: its
# main() stages 32-bit results (s1/s2 and the per-call return feeds) into
# 64-bit accumulator/call-argument relays while those values spill to 4-byte
# slots — the S64-relay reads the memory-operand substitution cannot
# express. Before the window allocator's narrow-slot promotion this
# replayed windows through the text path (MI-FALLBACK).
#
# Mutation-verified 2026-09-06: with the promotion disabled
# (classify_window blinded to small slots) this file reintroduces 3
# MI-FALLBACK windows and this check fails; execution stays correct (the
# replay is the fail-safe), which is exactly why the structural gate — not
# an output diff — is the regression lock for this class.
set -u
CCC=${CCC:-./target/fastbuild/lccc}
SRC=$(dirname "$0")/accumulator_reassociation.c

OUT=$(CCC_MI_DEBUG=1 CCC_MI_STREAM=1 "$CCC" -O2 -S "$SRC" -o /tmp/mwa_wc.$$ 2>&1)
rm -f /tmp/mwa_wc.$$
if ! grep -q "MI-STREAM fn=main" <<< "$OUT"; then
    echo "FAIL: accumulator main did not flow through MachInst at all"
    exit 1
fi
if grep -q "MI-FALLBACK" <<< "$OUT"; then
    echo "FAIL: accumulator main replayed a window (MI-FALLBACK present; narrow-slot promotion regression?)"
    exit 1
fi
exit 0
