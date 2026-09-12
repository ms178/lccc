#!/usr/bin/env bash
# Compare-replay home-rewrite gate (kernel 6.18.50 kernel/irq/affinity.c
# get_pcore_mask ICE):
#
#   1. ICE check — the multi-select compare-replay must never leave a
#      register-homed operand without a readable location. The failure
#      mode before the fix: the RA's latch/phi-web coalescing homes the
#      guarded-update Select result and the loop-carried accumulator in
#      ONE register; the first select's cmov rewrites the shared home
#      while the accumulator is still live for the second select's
#      re-emitted compare, and the operand load dies with
#      "x86 codegen: value N has no register, stack slot, Copy, or
#       GlobalAddr definition" (kernel build abort at kernel/irq/
#      affinity.c). A compile-time panic is a hard FAIL here.
#
#   2. Differential correctness — the guarded-update pairs must produce
#      GCC-identical results (the silent variant of the same bug
#      compares against the WRONG accumulator: a stale or overwritten
#      home read by the replayed compare).
#
# The C file mirrors get_pcore_mask's frequency fallback (sibling-union
# loop, the `if (f > max_freq && f != 0) { max_freq = f; max_freq_cpu =
# cpu; }` pair, the threshold second loop, the init-flag envelope) with
# kernel dependencies stubbed, so the shape survives without a kernel
# tree.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
tmp=${TMPDIR:-/tmp}/lccc-replay-gap.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"

src=$(dirname "$0")/replay_gap_home_rewrite.c

# 1. ICE / compile gate at the kernel's optimization shape.
"$CCC" -O2 -mcmodel=kernel -fno-omit-frame-pointer -mno-red-zone \
    -o "$tmp/t.bin" "$src" 2>"$tmp/err.log"
if grep -qE "internal error|panicked at" "$tmp/err.log"; then
    echo "FAIL: compare-replay left a value homeless (ICE)" >&2
    cat "$tmp/err.log" >&2
    exit 1
fi
"$tmp/t.bin" > "$tmp/lccc.out"
lccc_rc=$?

# 2. Differential against the GCC oracle (same source, same result).
gcc -O2 -o "$tmp/ref.bin" "$src"
"$tmp/ref.bin" > "$tmp/gcc.out"
gcc_rc=$?

if [[ $lccc_rc -ne $gcc_rc ]]; then
    echo "FAIL: exit status mismatch lccc=$lccc_rc gcc=$gcc_rc" >&2
    diff -u "$tmp/gcc.out" "$tmp/lccc.out" >&2 || true
    exit 1
fi
if ! diff -u "$tmp/gcc.out" "$tmp/lccc.out"; then
    echo "FAIL: compare-replay home-rewrite changed results" >&2
    exit 1
fi

# 3. The audit must stay armed: an operand whose gap contains a sibling
#    definition must NOT be trusted. Instrument via the debug env var on
#    the shape that historically coalesced in the kernel build (best
#    effort: the synthetic may coalesce or not depending on pressure, so
#    this only asserts no internal error under the replay machinery,
#    keeping the compile deterministic across RA evolutions).
LCCC_DEBUG_REPLAY=1 "$CCC" -O2 -o "$tmp/t2.bin" "$src" 2>/dev/null

echo "PASS: replay-gap home-rewrite (ICE + differential vs GCC)"
