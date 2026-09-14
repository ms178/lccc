#!/usr/bin/env bash
# RA-GLA-03: the Speed-tier reach band is derived per target from the buyable
# callee-saved GPR set and validated by measured fire-site frontiers:
#
#   aarch64 = 10 (x19-x28)   riscv64 = 6 (deliberately below the 11 s-GPRs)
#   x86-64  = 6 (rbx,rbp,r12-r15)        i686 = 2 (esi/edi under PIC)
#
# This is an *assembly/plan* pin: no qemu execution is required.
#
#  1. AArch64 default planning must use reach=10, which is what removes the
#     const slot home from the N=17 matmul prologue (frame 128 -> 112; the
#     hot FP loops are untouched). Forcing CCC_GLA_REACH=6 must restore the
#     wider frame, proving the number comes from the target derivation rather
#     than an x86 literal leaking across backends.
#
#  2. RISC-V default planning must stay at reach=6: on double_reduction the
#     three global bases are `covers_reachable=false` at the default and zero
#     edits are applied; forcing reach=11 admits them, and the register-
#     allocator cascade then remats the LCG constants *inside* the hot loop
#     (304 insns vs GCC's 74 — moving away from the oracle). This pin fails
#     loudly if a future "ABI says 11" change reintroduces that regression.
#
#  3. The x86-64 host default remains reach=6.
set -euo pipefail
repo=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
arm_cc=$repo/target/fastbuild/lccc-arm
rv_cc=$repo/target/fastbuild/lccc-riscv
x64_cc=${CCC:-$repo/target/fastbuild/lccc}
matmul=$repo/tests/regression/vectorize_matmul_n17.c
dred=$repo/tests/benchmark/programs/double_reduction.c
nbody=$repo/tests/benchmark/programs/nbody.c
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# Cross binaries are only present after the full fastbuild; hosts without the
# cross toolchain still run the x86-64 leg. Missing pieces skip, never fail.
arm_inc=""; rv_inc=""
if command -v aarch64-linux-gnu-gcc >/dev/null 2>&1; then
  arm_inc=$(aarch64-linux-gnu-gcc -print-file-name=include)
fi
if command -v riscv64-linux-gnu-gcc >/dev/null 2>&1; then
  rv_inc=$(riscv64-linux-gnu-gcc -print-file-name=include)
fi
fail=0
# Hermetic: an ambient A/B override must not change the derived defaults this
# test pins (the Rust unit tests skip under CCC_GLA_REACH for the same reason).
unset CCC_GLA_REACH

if [[ -x $arm_cc && -n $arm_inc ]]; then
  # (1) AArch64: reach=10 by default.
  env -u CCC_GLA_REACH CCC_DEBUG_SPLIT=1 CCC_RA_GLOBAL_LOCATION=1 CCC_GLA_TRACE=1 \
    "$arm_cc" -I"$arm_inc" -O2 -S "$matmul" -o /dev/null \
    >"$tmp/arm-trace.txt" 2>&1
  grep -q "reach=10" "$tmp/arm-trace.txt" || {
    echo "FAIL: aarch64 default Speed reach band is not 10"; fail=1; }

  env -u CCC_GLA_REACH "$arm_cc" -I"$arm_inc" -O2 -S "$matmul" -o "$tmp/mm10.s"
  env CCC_GLA_REACH=6 "$arm_cc" -I"$arm_inc" -O2 -S "$matmul" -o "$tmp/mm6.s"
  grep -Eq 'stp[[:space:]]+x29, x30, \[sp, #-112\]!' "$tmp/mm10.s" || {
    echo "FAIL: aarch64 matmul frame should be 112 at band 10"; fail=1; }
  grep -Eq 'stp[[:space:]]+x29, x30, \[sp, #-128\]!' "$tmp/mm6.s" || {
    echo "FAIL: aarch64 matmul frame should be 128 at forced band 6"; fail=1; }
  echo "ok: aarch64 reach band 10 (matmul frame 128 -> 112)"
else
  echo "SKIP: aarch64 cross compiler unavailable"
fi

if [[ -x $rv_cc && -n $rv_inc ]]; then
  # (2) RISC-V: reach=6 by default; the three bases are unreachable and the
  # function takes zero edits.
  env -u CCC_GLA_REACH CCC_DEBUG_SPLIT=1 CCC_RA_GLOBAL_LOCATION=1 CCC_GLA_TRACE=1 \
    "$rv_cc" -I"$rv_inc" -O2 -S "$dred" -o "$tmp/dr6.s" \
    >"$tmp/rv6-trace.txt" 2>&1
  grep -q "reach=6" "$tmp/rv6-trace.txt" || {
    echo "FAIL: riscv64 default Speed reach band is not 6"; fail=1; }
  if grep -q "covers_reachable=true b0..b2" "$tmp/rv6-trace.txt"; then
    echo "FAIL: riscv64 double_reduction bases reachable at band 6 (hot-loop cascade; RA-GLA-03)"; fail=1
  fi
  if grep -q "\[GLA\] main applied: [1-9]" "$tmp/rv6-trace.txt"; then
    echo "FAIL: riscv64 double_reduction applied GLA edits at band 6"; fail=1
  fi

  # Forcing 11 reopens the documented cliff: three weighted-1 global bases
  # become reachable and are applied, and three large li pairs move into the
  # LCG loop.
  env CCC_DEBUG_SPLIT=1 CCC_RA_GLOBAL_LOCATION=1 CCC_GLA_REACH=11 CCC_GLA_TRACE=1 \
    "$rv_cc" -I"$rv_inc" -O2 -S "$dred" -o "$tmp/dr11.s" \
    >"$tmp/rv11-trace.txt" 2>&1
  grep -q "applied: 3 values (3 remat" "$tmp/rv11-trace.txt" || {
    echo "FAIL: forced riscv64 band 11 no longer admits the 3 bases (RA-GLA-03 corpus changed - re-triage)"; fail=1; }
  echo "ok: riscv64 reach band 6 (band-11 hot-loop cliff pinned)"
else
  echo "SKIP: riscv64 cross compiler unavailable"
fi

# (3) x86-64 host derivation remains 6.
if [[ -x $x64_cc ]]; then
  env -u CCC_GLA_REACH CCC_DEBUG_SPLIT=1 CCC_RA_GLOBAL_LOCATION=1 CCC_GLA_TRACE=1 \
    "$x64_cc" -O2 -S "$nbody" -o /dev/null >"$tmp/x64-trace.txt" 2>&1
  grep -q "reach=6" "$tmp/x64-trace.txt" || {
    echo "FAIL: x86-64 default Speed reach band is not 6"; fail=1; }
  echo "ok: x86-64 reach band 6"
fi

if [[ $fail -ne 0 ]]; then exit 1; fi
echo "cross reach-band pins pass"
